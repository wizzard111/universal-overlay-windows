//! NVIDIA sensors through NVML, shipped with the driver on Windows
//! (nvml.dll) and Linux (libnvidia-ml.so.1). Loaded at runtime.

use super::{GpuSource, Snapshot};
use libloading::Library;
use std::ffi::c_void;
use uo_common::sensor::SensorId;

type Dev = *mut c_void;
type FnInit = unsafe extern "C" fn() -> i32;
type FnHandle = unsafe extern "C" fn(u32, *mut Dev) -> i32;
type FnName = unsafe extern "C" fn(Dev, *mut u8, u32) -> i32;
type FnUtil = unsafe extern "C" fn(Dev, *mut [u32; 2]) -> i32;
type FnU32Arg = unsafe extern "C" fn(Dev, u32, *mut u32) -> i32;
type FnU32 = unsafe extern "C" fn(Dev, *mut u32) -> i32;
type FnMem = unsafe extern "C" fn(Dev, *mut [u64; 3]) -> i32;

pub struct Nvml {
    _lib: Library,
    dev: Dev,
    name: String,
    util: Option<FnUtil>,
    temp: Option<FnU32Arg>,
    clock: Option<FnU32Arg>,
    mem: Option<FnMem>,
    power: Option<FnU32>,
    fan: Option<FnU32>,
}

const CANDIDATES: &[&str] = &[
    "nvml.dll",
    "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll",
];

impl Nvml {
    pub fn new() -> Option<Self> {
        unsafe {
            let lib = CANDIDATES.iter().find_map(|p| Library::new(*p).ok())?;
            fn sym<T: Copy>(lib: &Library, name: &[u8]) -> Option<T> {
                unsafe { lib.get::<T>(name).ok().map(|s| *s) }
            }
            let init: FnInit = sym(&lib, b"nvmlInit_v2\0")?;
            let handle: FnHandle = sym(&lib, b"nvmlDeviceGetHandleByIndex_v2\0")?;
            if init() != 0 {
                return None;
            }
            let mut dev: Dev = std::ptr::null_mut();
            if handle(0, &mut dev) != 0 {
                return None;
            }
            let mut name = [0u8; 96];
            if let Some(f) = sym::<FnName>(&lib, b"nvmlDeviceGetName\0") {
                f(dev, name.as_mut_ptr(), name.len() as u32);
            }
            let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            Some(Self {
                dev,
                name: String::from_utf8_lossy(&name[..end]).into_owned(),
                util: sym(&lib, b"nvmlDeviceGetUtilizationRates\0"),
                temp: sym(&lib, b"nvmlDeviceGetTemperature\0"),
                clock: sym(&lib, b"nvmlDeviceGetClockInfo\0"),
                mem: sym(&lib, b"nvmlDeviceGetMemoryInfo\0"),
                power: sym(&lib, b"nvmlDeviceGetPowerUsage\0"),
                fan: sym(&lib, b"nvmlDeviceGetFanSpeed\0"),
                _lib: lib,
            })
        }
    }
}

impl GpuSource for Nvml {
    fn name(&self) -> String {
        format!("{} (NVML)", self.name)
    }

    fn poll(&mut self, s: &mut Snapshot) {
        unsafe {
            let d = self.dev;
            if let Some(f) = self.util {
                let mut u = [0u32; 2];
                s.set(
                    SensorId::GpuLoad,
                    (f(d, &mut u) == 0).then_some(u[0] as f64),
                );
            }
            if let Some(f) = self.temp {
                let mut t = 0;
                s.set(
                    SensorId::GpuTemp,
                    (f(d, 0, &mut t) == 0).then_some(t as f64),
                );
            }
            if let Some(f) = self.clock {
                let (mut g, mut m) = (0, 0);
                s.set(
                    SensorId::GpuClock,
                    (f(d, 0, &mut g) == 0).then_some(g as f64),
                );
                s.set(
                    SensorId::GpuMemClock,
                    (f(d, 2, &mut m) == 0).then_some(m as f64),
                );
            }
            if let Some(f) = self.mem {
                let mut m = [0u64; 3]; // total, free, used
                s.set(
                    SensorId::GpuVram,
                    (f(d, &mut m) == 0).then_some(m[2] as f64 / (1024.0 * 1024.0)),
                );
            }
            if let Some(f) = self.power {
                let mut mw = 0;
                s.set(
                    SensorId::GpuPower,
                    (f(d, &mut mw) == 0).then_some(mw as f64 / 1000.0),
                );
            }
            if let Some(f) = self.fan {
                let mut p = 0;
                s.set(SensorId::GpuFanPct, (f(d, &mut p) == 0).then_some(p as f64));
            }
        }
    }
}
