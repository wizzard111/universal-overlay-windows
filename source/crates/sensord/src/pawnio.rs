//! CPU temperature on Windows via PawnIO — a small signed kernel driver
//! (https://pawnio.eu, used by LibreHardwareMonitor and FanControl) that
//! executes verified bytecode modules. We use two official modules:
//!   * AMDFamily17.bin — `ioctl_read_smn` (Zen 1…5: families 17h, 19h, 1Ah)
//!   * IntelMSR.bin    — `ioctl_read_msr`
//!
//! One reading is a single DeviceIoControl round trip (microseconds).
//!
//! The installer installs the driver and puts the module files into
//! `<install dir>\pawnio\`. Opening the driver needs full rights, which is
//! why this code lives in the `uo-sensord` service and not in the overlay.

use std::ffi::{c_char, c_void};
use std::path::{Path, PathBuf};
use windows_sys::Win32::System::Registry::*;

type Handle = *mut c_void;
type FnOpen = unsafe extern "system" fn(*mut Handle) -> i32;
type FnLoad = unsafe extern "system" fn(Handle, *const u8, usize) -> i32;
type FnExec = unsafe extern "system" fn(
    Handle,
    *const c_char,
    *const u64,
    usize,
    *mut u64,
    usize,
    *mut usize,
) -> i32;
type FnClose = unsafe extern "system" fn(Handle) -> i32;

#[derive(Clone, Copy)]
enum Kind {
    Amd { ccd_base: Option<u32> },
    Intel,
}

pub struct CpuTemp {
    _lib: libloading::Library,
    h: Handle,
    exec: FnExec,
    close: FnClose,
    kind: Kind,
    tj_max: f64,
}

/// Why the CPU temperature is unavailable (shown in the probe).
#[derive(Debug)]
pub enum Unavailable {
    UnsupportedCpu,
    NoDriver,
    NoModule,
    /// The driver refused: the process lacks full rights.
    AccessDenied,
    Failed,
}

fn cpuid(leaf: u32) -> [u32; 4] {
    #[cfg(target_arch = "x86_64")]
    {
        let r = std::arch::x86_64::__cpuid(leaf);
        [r.eax, r.ebx, r.ecx, r.edx]
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = leaf;
        [0; 4]
    }
}

/// (vendor, family, model) from CPUID.
fn cpu_ident() -> (String, u32, u32) {
    let v = cpuid(0);
    let mut vendor = Vec::with_capacity(12);
    for r in [v[1], v[3], v[2]] {
        vendor.extend_from_slice(&r.to_le_bytes());
    }
    let f = cpuid(1)[0];
    let base_family = (f >> 8) & 0xF;
    let family = if base_family == 0xF {
        base_family + ((f >> 20) & 0xFF)
    } else {
        base_family
    };
    let model = ((f >> 4) & 0xF)
        | if base_family >= 0x6 {
            ((f >> 16) & 0xF) << 4
        } else {
            0
        };
    (String::from_utf8_lossy(&vendor).into_owned(), family, model)
}

fn reg_string(sub: &str, value: &str) -> Option<String> {
    let sub: Vec<u16> = sub.encode_utf16().chain(Some(0)).collect();
    let val: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sub.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
        )
    };
    if r != 0 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(len as usize / 2)]);
    Some(s.trim_end_matches('\0').trim_matches('"').to_string())
}

/// Locations where PawnIOLib.dll may be installed.
fn library_candidates() -> Vec<PathBuf> {
    let key = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PawnIO";
    let mut v = Vec::new();
    if let Some(dir) = reg_string(key, "InstallLocation") {
        v.push(PathBuf::from(dir).join("PawnIOLib.dll"));
    }
    if let Some(un) = reg_string(key, "UninstallString") {
        if let Some(dir) = Path::new(un.split(".exe").next().unwrap_or("")).parent() {
            v.push(dir.join("PawnIOLib.dll"));
        }
    }
    let pf = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
    v.push(pf.join("PawnIO").join("PawnIOLib.dll"));
    v.push(PathBuf::from("PawnIOLib.dll"));
    v
}

pub fn module_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("pawnio")))
        .unwrap_or_else(|| PathBuf::from("pawnio"))
}

const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;

impl CpuTemp {
    pub fn new() -> Result<Self, Unavailable> {
        let (vendor, family, model) = cpu_ident();
        let (kind, module) = match vendor.as_str() {
            "AuthenticAMD" if matches!(family, 0x17 | 0x19 | 0x1A) => {
                // CCD temperature register base, per LibreHardwareMonitor.
                let ccd_base = match (family, model) {
                    (0x19, 0x61) | (0x19, 0x44) => Some(0x0005_9b08),
                    (0x1A, m) if m <= 0x2F => Some(0x0005_99F0),
                    _ => Some(0x0005_9954),
                };
                (Kind::Amd { ccd_base }, "AMDFamily17.bin")
            }
            "GenuineIntel" => (Kind::Intel, "IntelMSR.bin"),
            _ => return Err(Unavailable::UnsupportedCpu),
        };
        let lib = library_candidates()
            .into_iter()
            .find_map(|p| unsafe { libloading::Library::new(&p).ok() })
            .ok_or(Unavailable::NoDriver)?;
        let path = module_dir().join(module);
        let blob = std::fs::read(&path).map_err(|_| Unavailable::NoModule)?;
        unsafe {
            let open: FnOpen = *lib
                .get(b"pawnio_open\0")
                .map_err(|_| Unavailable::NoDriver)?;
            let load: FnLoad = *lib
                .get(b"pawnio_load\0")
                .map_err(|_| Unavailable::NoDriver)?;
            let exec: FnExec = *lib
                .get(b"pawnio_execute\0")
                .map_err(|_| Unavailable::NoDriver)?;
            let close: FnClose = *lib
                .get(b"pawnio_close\0")
                .map_err(|_| Unavailable::NoDriver)?;
            let mut h: Handle = std::ptr::null_mut();
            let r = open(&mut h);
            if r < 0 {
                return Err(if r == E_ACCESSDENIED {
                    Unavailable::AccessDenied
                } else {
                    Unavailable::Failed
                });
            }
            let r = load(h, blob.as_ptr(), blob.len());
            if r < 0 {
                close(h);
                return Err(Unavailable::Failed);
            }
            let mut t = CpuTemp {
                _lib: lib,
                h,
                exec,
                close,
                kind,
                tj_max: 100.0,
            };
            if let Kind::Intel = kind {
                // IA32_TEMPERATURE_TARGET bits 23:16 = TjMax.
                if let Some(v) = t.call("ioctl_read_msr", 0x1A2) {
                    let tj = ((v >> 16) & 0xFF) as f64;
                    if tj > 0.0 {
                        t.tj_max = tj;
                    }
                }
            }
            Ok(t)
        }
    }

    fn call(&self, name: &str, arg: u64) -> Option<u64> {
        let mut n = [0u8; 32];
        n[..name.len()].copy_from_slice(name.as_bytes());
        let input = [arg];
        let mut out = [0u64; 1];
        let mut got = 0usize;
        let r = unsafe {
            (self.exec)(
                self.h,
                n.as_ptr() as *const c_char,
                input.as_ptr(),
                1,
                out.as_mut_ptr(),
                1,
                &mut got,
            )
        };
        (r >= 0 && got >= 1).then_some(out[0])
    }

    /// Package / Tctl temperature in °C.
    pub fn read(&self) -> Option<f64> {
        match self.kind {
            Kind::Amd { .. } => {
                // THM_TCON_CUR_TMP: bits 31:21 = temp × 8; "range select"
                // (bit 19, or bits 17:16 == 3) means a −49 °C offset.
                let raw = self.call("ioctl_read_smn", 0x0005_9800)? as u32;
                let mut t = (raw >> 21) as f64 * 0.125;
                if raw & 0x8_0000 != 0 || raw & 0x3_0000 == 0x3_0000 {
                    t -= 49.0;
                }
                Some(t)
            }
            Kind::Intel => {
                // IA32_PACKAGE_THERM_STATUS: bit 31 valid, bits 22:16 = distance to TjMax.
                let v = self.call("ioctl_read_msr", 0x1B1)?;
                (v & 0x8000_0000 != 0).then_some(self.tj_max - ((v >> 16) & 0x7F) as f64)
            }
        }
    }

    /// AMD only: hottest CCD temperature (°C), if the CPU reports CCDs.
    pub fn read_ccd_max(&self) -> Option<f64> {
        let Kind::Amd {
            ccd_base: Some(base),
        } = self.kind
        else {
            return None;
        };
        (0..8u32)
            .filter_map(|i| {
                let raw = self.call("ioctl_read_smn", (base + i * 4) as u64)? as u32;
                // As in LibreHardwareMonitor: value = (raw & 0xFFF) × 0.125 − 305, 0 = absent CCD.
                let v = raw & 0xFFF;
                (v > 0).then_some(v as f64 * 0.125 - 305.0)
            })
            .filter(|t| (-20.0..150.0).contains(t))
            .fold(None, |m: Option<f64>, t| Some(m.map_or(t, |m| m.max(t))))
    }

    /// Plausible package temperature only (filters glitches).
    pub fn read_checked(&self) -> Option<f64> {
        self.read().filter(|t| (-20.0..150.0).contains(t))
    }
}

impl Unavailable {
    pub fn status(&self) -> i32 {
        use uo_common::sensord::*;
        match self {
            Unavailable::UnsupportedCpu => STATUS_UNSUPPORTED_CPU,
            Unavailable::NoDriver => STATUS_NO_DRIVER,
            Unavailable::NoModule => STATUS_NO_MODULE,
            Unavailable::AccessDenied | Unavailable::Failed => STATUS_FAILED,
        }
    }
}

impl Drop for CpuTemp {
    fn drop(&mut self) {
        unsafe { (self.close)(self.h) };
    }
}
