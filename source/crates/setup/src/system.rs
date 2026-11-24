//! Windows plumbing for the installer: registry, service, processes,
//! user group, launching. Plain Win32 calls, no scripting.

use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use uo_common::sensord::{SERVICE_DISPLAY, SERVICE_NAME};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::NetworkManagement::NetManagement::*;
use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
use windows_sys::Win32::System::Console::SetConsoleTitleW;
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Services::*;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, ShellExecuteW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, SW_HIDE, SW_SHOWNORMAL,
};

const UNINSTALL_KEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\UniversalOverlay";
const PAWNIO_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PawnIO";
const RUN_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Universal Overlay";

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn env_path(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fallback))
}

pub fn program_data() -> PathBuf {
    env_path("ProgramData", "C:\\ProgramData")
}

pub fn default_install_dir() -> PathBuf {
    env_path("ProgramFiles", "C:\\Program Files").join("Universal Overlay")
}

pub fn start_menu_dir() -> PathBuf {
    program_data().join("Microsoft\\Windows\\Start Menu\\Programs")
}

pub fn local_time() -> String {
    let mut t = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

pub fn set_console_title(t: &str) {
    unsafe { SetConsoleTitleW(wide(t).as_ptr()) };
}

// ---------------------------------------------------------------- registry

fn reg_get(root: HKEY, key: &str, value: &str) -> Option<String> {
    let mut buf = [0u16; 2048];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            root,
            wide(key).as_ptr(),
            wide(value).as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    (r == 0).then(|| {
        String::from_utf16_lossy(&buf[..len as usize / 2])
            .trim_end_matches('\0')
            .to_string()
    })
}

fn reg_key_exists(key: &str) -> bool {
    let mut h: HKEY = null_mut();
    let r = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, wide(key).as_ptr(), 0, KEY_READ, &mut h) };
    if r == 0 {
        unsafe { RegCloseKey(h) };
    }
    r == 0
}

struct Key(HKEY);

impl Key {
    fn create(key: &str) -> Result<Key, String> {
        let mut h: HKEY = null_mut();
        let r = unsafe {
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                wide(key).as_ptr(),
                0,
                null(),
                0,
                KEY_WRITE,
                null(),
                &mut h,
                null_mut(),
            )
        };
        if r == 0 {
            Ok(Key(h))
        } else {
            Err(format!("registry {key}: {r}"))
        }
    }

    fn set_str(&self, name: &str, v: &str) {
        let w = wide(v);
        unsafe {
            RegSetValueExW(
                self.0,
                wide(name).as_ptr(),
                0,
                REG_SZ,
                w.as_ptr().cast(),
                (w.len() * 2) as u32,
            )
        };
    }

    fn set_dword(&self, name: &str, v: u32) {
        unsafe {
            RegSetValueExW(
                self.0,
                wide(name).as_ptr(),
                0,
                REG_DWORD,
                (&v as *const u32).cast(),
                4,
            )
        };
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

pub fn registered_install_dir() -> Option<PathBuf> {
    reg_get(HKEY_LOCAL_MACHINE, UNINSTALL_KEY, "InstallLocation")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

pub fn register_uninstall(dir: &Path, version: &str, size: u64) -> Result<(), String> {
    let k = Key::create(UNINSTALL_KEY)?;
    let un = dir.join("uninstall.exe");
    k.set_str("DisplayName", "Universal Overlay");
    k.set_str("DisplayVersion", version);
    k.set_str("Publisher", "Universal Overlay");
    k.set_str(
        "DisplayIcon",
        &format!("{},0", dir.join("universal-overlay.exe").display()),
    );
    k.set_str("InstallLocation", &dir.display().to_string());
    k.set_str("UninstallString", &format!("\"{}\"", un.display()));
    k.set_str(
        "QuietUninstallString",
        &format!("\"{}\" --quiet", un.display()),
    );
    k.set_dword("EstimatedSize", (size / 1024) as u32);
    k.set_dword("NoModify", 1);
    k.set_dword("NoRepair", 1);
    Ok(())
}

pub fn unregister_uninstall() {
    unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, wide(UNINSTALL_KEY).as_ptr()) };
    unsafe { RegDeleteKeyW(HKEY_LOCAL_MACHINE, wide(UNINSTALL_KEY).as_ptr()) };
}

/// Autostart for every user (HKLM Run); `None` removes it.
pub fn set_autostart(exe: Option<&Path>) -> Result<(), String> {
    match exe {
        Some(p) => {
            Key::create(RUN_KEY)?.set_str(RUN_VALUE, &format!("\"{}\" --background", p.display()))
        }
        None => unsafe {
            RegDeleteKeyValueW(
                HKEY_LOCAL_MACHINE,
                wide(RUN_KEY).as_ptr(),
                wide(RUN_VALUE).as_ptr(),
            );
        },
    }
    Ok(())
}

// ---------------------------------------------------------------- processes

fn for_each_process(mut f: impl FnMut(&PROCESSENTRY32W, String)) {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let n = e
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(e.szExeFile.len());
            f(
                &e,
                String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase(),
            );
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
}

/// Ask the overlay to quit, then end whatever is left of our programs.
pub fn stop_overlay(names: &[&str]) {
    unsafe {
        let w = FindWindowW(wide(uo_common::OVERLAY_WINDOW_CLASS).as_ptr(), null());
        if !w.is_null() {
            PostMessageW(w, uo_common::MSG_QUIT, 0, 0);
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if FindWindowW(wide(uo_common::OVERLAY_WINDOW_CLASS).as_ptr(), null()).is_null() {
                    break;
                }
            }
        }
    }
    let me = std::process::id();
    for_each_process(|e, name| {
        if e.th32ProcessID != me && names.iter().any(|n| *n == name) {
            unsafe {
                let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, e.th32ProcessID);
                if !h.is_null() {
                    TerminateProcess(h, 0);
                    WaitForSingleObject(h, 2000);
                    CloseHandle(h);
                }
            }
        }
    });
}

/// Run a program and wait for its exit code.
pub fn run_wait(exe: &Path, args: &str) -> Result<u32, String> {
    let file = wide(&exe.to_string_lossy());
    let params = wide(args);
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 || info.hProcess.is_null() {
            return Err(format!("could not start {}", exe.display()));
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        Ok(code)
    }
}

/// Start a program as the signed-in user (not elevated): Explorer opens it.
pub fn launch_as_user(exe: &Path) {
    let args = wide(&format!("\"{}\"", exe.display()));
    unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            wide("explorer.exe").as_ptr(),
            args.as_ptr(),
            null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Wait (up to `ms`) until process `pid` has exited.
pub fn wait_for_exit(pid: u32, ms: u32) {
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, ms);
            CloseHandle(h);
        }
    }
}

pub fn delete_at_reboot(p: &Path) {
    unsafe {
        MoveFileExW(
            wide(&p.to_string_lossy()).as_ptr(),
            null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        )
    };
}

// ---------------------------------------------------------------- PawnIO

pub fn pawnio_installed() -> bool {
    reg_key_exists(PAWNIO_KEY)
}

pub fn pawnio_uninstall() {
    if let Some(un) = reg_get(HKEY_LOCAL_MACHINE, PAWNIO_KEY, "UninstallString") {
        let exe = un
            .trim()
            .trim_start_matches('"')
            .split('"')
            .next()
            .unwrap_or("")
            .to_string();
        let exe = if exe.to_lowercase().ends_with(".exe") {
            exe
        } else {
            un.split(".exe").next().unwrap_or("").to_string() + ".exe"
        };
        let _ = run_wait(Path::new(&exe), "-uninstall -silent");
    }
}

// ---------------------------------------------------------------- service

struct Sc(SC_HANDLE);

impl Drop for Sc {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0) };
    }
}

fn scm() -> Result<Sc, String> {
    let h = unsafe { OpenSCManagerW(null(), null(), SC_MANAGER_ALL_ACCESS) };
    if h.is_null() {
        Err(format!("service manager: {}", unsafe { GetLastError() }))
    } else {
        Ok(Sc(h))
    }
}

fn open_service(m: &Sc) -> Option<Sc> {
    let h = unsafe { OpenServiceW(m.0, wide(SERVICE_NAME).as_ptr(), SERVICE_ALL_ACCESS) };
    (!h.is_null()).then_some(Sc(h))
}

fn wait_stopped(s: &Sc) {
    for _ in 0..50 {
        let mut st: SERVICE_STATUS = unsafe { std::mem::zeroed() };
        if unsafe { QueryServiceStatus(s.0, &mut st) } == 0 || st.dwCurrentState == SERVICE_STOPPED
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

pub fn service_stop() {
    if let Ok(m) = scm() {
        if let Some(s) = open_service(&m) {
            let mut st: SERVICE_STATUS = unsafe { std::mem::zeroed() };
            unsafe { ControlService(s.0, SERVICE_CONTROL_STOP, &mut st) };
            wait_stopped(&s);
        }
    }
}

/// Create or update the sensor service (LocalSystem, automatic) and start it.
pub fn service_install(exe: &Path) -> Result<(), String> {
    let m = scm()?;
    let bin = wide(&format!("\"{}\"", exe.display()));
    let display = wide(SERVICE_DISPLAY);
    let s = match open_service(&m) {
        Some(s) => {
            unsafe {
                ChangeServiceConfigW(
                    s.0,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_AUTO_START,
                    SERVICE_ERROR_NORMAL,
                    bin.as_ptr(),
                    null(),
                    null_mut(),
                    null(),
                    null(),
                    null(),
                    display.as_ptr(),
                )
            };
            s
        }
        None => {
            let h = unsafe {
                CreateServiceW(
                    m.0,
                    wide(SERVICE_NAME).as_ptr(),
                    display.as_ptr(),
                    SERVICE_ALL_ACCESS,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_AUTO_START,
                    SERVICE_ERROR_NORMAL,
                    bin.as_ptr(),
                    null(),
                    null_mut(),
                    null(),
                    null(),
                    null(),
                )
            };
            if h.is_null() {
                return Err(format!("CreateService: {}", unsafe { GetLastError() }));
            }
            Sc(h)
        }
    };
    let mut desc_text = wide("Reads the CPU temperature through the PawnIO driver for Universal Overlay. Works only while the overlay requests data.");
    let desc = SERVICE_DESCRIPTIONW {
        lpDescription: desc_text.as_mut_ptr(),
    };
    unsafe {
        ChangeServiceConfig2W(
            s.0,
            SERVICE_CONFIG_DESCRIPTION,
            (&desc as *const SERVICE_DESCRIPTIONW).cast(),
        )
    };
    if unsafe { StartServiceW(s.0, 0, null()) } == 0 {
        let e = unsafe { GetLastError() };
        if e != ERROR_SERVICE_ALREADY_RUNNING {
            return Err(format!("StartService: {e}"));
        }
    }
    Ok(())
}

pub fn service_remove() {
    if let Ok(m) = scm() {
        if let Some(s) = open_service(&m) {
            let mut st: SERVICE_STATUS = unsafe { std::mem::zeroed() };
            unsafe { ControlService(s.0, SERVICE_CONTROL_STOP, &mut st) };
            wait_stopped(&s);
            unsafe { DeleteService(s.0) };
        }
    }
}

// ---------------------------------------------------------------- user group

/// SID of the user signed in to this session (owner of explorer.exe), so
/// the right account is used even when an administrator elevated us.
fn session_user_sid() -> Option<Vec<u8>> {
    let mut my_session = 0;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut my_session) };
    let mut result = None;
    for_each_process(|e, name| {
        if result.is_some() || name != "explorer.exe" {
            return;
        }
        let mut s = u32::MAX;
        unsafe { ProcessIdToSessionId(e.th32ProcessID, &mut s) };
        if s != my_session {
            return;
        }
        unsafe {
            let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, e.th32ProcessID);
            if p.is_null() {
                return;
            }
            let mut tok: HANDLE = null_mut();
            if OpenProcessToken(p, TOKEN_QUERY, &mut tok) != 0 {
                let mut buf = vec![0u8; 256];
                let mut len = 0;
                if GetTokenInformation(
                    tok,
                    TokenUser,
                    buf.as_mut_ptr().cast(),
                    buf.len() as u32,
                    &mut len,
                ) != 0
                {
                    let tu = &*(buf.as_ptr() as *const TOKEN_USER);
                    let n = GetLengthSid(tu.User.Sid) as usize;
                    result = Some(std::slice::from_raw_parts(tu.User.Sid as *const u8, n).to_vec());
                }
                CloseHandle(tok);
            }
            CloseHandle(p);
        }
    });
    result
}

/// Add the signed-in user to "Performance Log Users" (S-1-5-32-559), which
/// lets a normal program read frame-present ETW events. Ok(false) = already
/// a member.
pub fn add_to_performance_log_users() -> Result<bool, String> {
    let mut user = session_user_sid().ok_or("signed-in user not found")?;
    unsafe {
        let mut group_sid: PSID = null_mut();
        if ConvertStringSidToSidW(wide("S-1-5-32-559").as_ptr(), &mut group_sid) == 0 {
            return Err("group SID".into());
        }
        let mut name = [0u16; 256];
        let mut name_len = name.len() as u32;
        let mut domain = [0u16; 256];
        let mut domain_len = domain.len() as u32;
        let mut use_ = 0;
        let ok = LookupAccountSidW(
            null(),
            group_sid,
            name.as_mut_ptr(),
            &mut name_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut use_,
        );
        LocalFree(group_sid as HLOCAL);
        if ok == 0 {
            return Err(format!("group name: {}", GetLastError()));
        }
        let member = LOCALGROUP_MEMBERS_INFO_0 {
            lgrmi0_sid: user.as_mut_ptr().cast(),
        };
        let r = NetLocalGroupAddMembers(
            null(),
            name.as_ptr(),
            0,
            (&member as *const LOCALGROUP_MEMBERS_INFO_0).cast(),
            1,
        );
        match r {
            0 => Ok(true),
            ERROR_MEMBER_IN_ALIAS => Ok(false),
            e => Err(format!("code {e}")),
        }
    }
}
