//! uo-sensord — Universal Overlay sensor service (Windows).
//!
//! A tiny service whose only job is reading the CPU temperature through the
//! signed PawnIO driver, which requires full rights. Keeping this here lets
//! the overlay itself run as a normal user. Protocol: see
//! `uo_common::sensord`. The service reads the sensor only when the overlay
//! asks (once per sensor tick while the overlay is visible); otherwise it
//! sleeps in the kernel and uses no CPU.
//!
//!   uo-sensord.exe            started by the Service Control Manager
//!   uo-sensord.exe --console  run in the foreground (diagnostics)
#![windows_subsystem = "windows"]

mod pawnio;

use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use uo_common::sensord::*;
use windows_sys::core::PWSTR;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Services::*;
use windows_sys::Win32::System::Threading::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

static STOP: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());
static STATUS: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());

/// Security attributes from an SDDL string (kept alive for the process).
fn security(sddl: &str) -> SECURITY_ATTRIBUTES {
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    let w = wide(sddl);
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            w.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            null_mut(),
        )
    };
    SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    }
}

/// Serve readings until `stop` is signalled.
fn serve(stop: HANDLE) -> Result<(), String> {
    // SYSTEM and administrators: full access. Signed-in users: read the
    // section; set / wait on the events (EVENT_MODIFY_STATE | SYNCHRONIZE).
    let sa_map = security("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GR;;;AU)");
    let sa_evt = security("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x00100002;;;AU)");
    unsafe {
        let map = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            &sa_map,
            PAGE_READWRITE,
            0,
            SIZE as u32,
            wide(MAPPING).as_ptr(),
        );
        if map.is_null() {
            return Err(format!("CreateFileMapping: {}", GetLastError()));
        }
        let view = MapViewOfFile(map, FILE_MAP_WRITE, 0, 0, SIZE);
        if view.Value.is_null() {
            return Err(format!("MapViewOfFile: {}", GetLastError()));
        }
        let shared = &*(view.Value as *const Shared);
        let want = CreateEventW(&sa_evt, 0, 0, wide(EVENT_WANT).as_ptr());
        let done = CreateEventW(&sa_evt, 0, 0, wide(EVENT_DONE).as_ptr());
        if want.is_null() || done.is_null() {
            return Err(format!("CreateEvent: {}", GetLastError()));
        }

        let nan = f32::NAN.to_bits();
        let mut sensor = pawnio::CpuTemp::new();
        let mut last_init = std::time::Instant::now();
        shared.cpu_temp.store(nan, Ordering::Relaxed);
        shared.ccd_max.store(nan, Ordering::Relaxed);
        shared.magic.store(MAGIC, Ordering::Release);

        let handles = [stop, want];
        loop {
            let r = WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE);
            if r != WAIT_OBJECT_0 + 1 {
                break; // stop requested (or wait failed)
            }
            // PawnIO may have been installed after the service started.
            if sensor.is_err() && last_init.elapsed().as_secs() >= 10 {
                sensor = pawnio::CpuTemp::new();
                last_init = std::time::Instant::now();
            }
            match &sensor {
                Ok(t) => {
                    let temp = t.read_checked().map_or(nan, |v| (v as f32).to_bits());
                    let ccd = t.read_ccd_max().map_or(nan, |v| (v as f32).to_bits());
                    shared.cpu_temp.store(temp, Ordering::Relaxed);
                    shared.ccd_max.store(ccd, Ordering::Relaxed);
                    shared.status.store(STATUS_OK, Ordering::Relaxed);
                }
                Err(e) => shared.status.store(e.status(), Ordering::Relaxed),
            }
            shared.seq.fetch_add(1, Ordering::Release);
            SetEvent(done);
        }
        UnmapViewOfFile(view);
        CloseHandle(want);
        CloseHandle(done);
        CloseHandle(map);
    }
    Ok(())
}

fn report(state: SERVICE_STATUS_CURRENT_STATE) {
    let h = STATUS.load(Ordering::Relaxed);
    if h.is_null() {
        return;
    }
    let st = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        } else {
            0
        },
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: if state == SERVICE_RUNNING { 0 } else { 2000 },
    };
    unsafe { SetServiceStatus(h, &st) };
}

unsafe extern "system" fn handler(
    ctrl: u32,
    _t: u32,
    _d: *mut core::ffi::c_void,
    _c: *mut core::ffi::c_void,
) -> u32 {
    match ctrl {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            report(SERVICE_STOP_PENDING);
            SetEvent(STOP.load(Ordering::Relaxed));
            NO_ERROR
        }
        SERVICE_CONTROL_INTERROGATE => NO_ERROR,
        _ => ERROR_CALL_NOT_IMPLEMENTED,
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    let name = wide(SERVICE_NAME);
    let h = RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), null_mut());
    STATUS.store(h, Ordering::Relaxed);
    let stop = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
    STOP.store(stop, Ordering::Relaxed);
    report(SERVICE_RUNNING);
    let _ = serve(stop);
    report(SERVICE_STOPPED);
}

fn main() {
    if std::env::args().any(|a| a == "--console") {
        let stop = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if let Err(e) = serve(stop) {
            eprintln!("uo-sensord: {e}");
            std::process::exit(1);
        }
        return;
    }
    let mut name = wide(SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: null_mut(),
            lpServiceProc: None,
        },
    ];
    unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) };
}
