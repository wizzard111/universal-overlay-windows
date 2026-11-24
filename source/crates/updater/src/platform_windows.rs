//! Windows specifics of the updater.

use std::path::{Path, PathBuf};
use std::process::Command;
use uo_common::OVERLAY_EXE;

const UNINSTALL_KEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\UniversalOverlay";

pub fn no_window(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

fn reg_install_location() -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::*;
    let sub: Vec<u16> = UNINSTALL_KEY.encode_utf16().chain(Some(0)).collect();
    let val: Vec<u16> = "InstallLocation".encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sub.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    (r == 0).then(|| {
        PathBuf::from(String::from_utf16_lossy(&buf[..len as usize / 2]).trim_end_matches('\0'))
    })
}

/// Our own folder if it holds the program, else the registered installation.
pub fn install_dir() -> PathBuf {
    let own = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    if own.join(format!("{OVERLAY_EXE}.exe")).is_file() {
        return own;
    }
    reg_install_location().unwrap_or_else(|| {
        PathBuf::from(
            std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into()),
        )
        .join("Universal Overlay")
    })
}

pub fn version_file(dir: &Path) -> Option<PathBuf> {
    Some(dir.join("VERSION"))
}

/// A running .exe cannot be overwritten: continue from a copy in `work`.
pub fn restart_staged(work: &Path, repo: &str, install: &Path) -> Option<Result<String, String>> {
    let me = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return Some(Err(e.to_string())),
    };
    let copy = work.join("updater.exe");
    if let Err(e) = std::fs::copy(&me, &copy) {
        return Some(Err(e.to_string()));
    }
    let st = Command::new(&copy)
        .args(["--staged", "--repo", repo, "--install-dir"])
        .arg(install)
        .status();
    Some(match st {
        Ok(s) if s.success() => Ok(String::new()),
        Ok(_) => Err("обновление не удалось".into()),
        Err(e) => Err(e.to_string()),
    })
}

/// Run the new package's `install.exe --update`. Its manifest asks for
/// administrator rights, so it is started through ShellExecuteEx (the
/// normal UAC prompt) and we wait for it.
pub fn run_installer(pkg: &Path, install: &Path) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };
    let file = wide(&pkg.join("install.exe").to_string_lossy());
    let params = wide(&format!("--update --install-dir \"{}\"", install.display()));
    let verb = wide("open");
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut info) == 0 || info.hProcess.is_null() {
            return Err("установщик не запущен (отказано в правах администратора?)".into());
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if code == 0 {
            Ok(())
        } else {
            Err(format!("установщик завершился с кодом {code}"))
        }
    }
}

/// When started by double-click (own console), keep the window open.
pub fn pause_if_console() {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    let mut ids = [0u32; 4];
    let n = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) };
    if n == 1 && !std::env::args().any(|a| a == "--staged") {
        println!("Нажмите Enter, чтобы закрыть окно");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}
