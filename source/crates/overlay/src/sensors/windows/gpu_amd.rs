//! AMD Radeon sensors through the driver's ADL library (atiadlxx.dll),
//! the same PM-log telemetry Adrenalin reads. Loaded dynamically, so the
//! overlay runs fine on machines without an AMD driver.

use super::{GpuSource, Snapshot};
use std::ffi::c_void;
use uo_common::sensor::SensorId;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapAlloc};

const ADL_MAX_PATH: usize = 256;
const PMLOG_MAX: usize = 256;

#[repr(C)]
struct AdapterInfo {
    size: i32,
    adapter_index: i32,
    udid: [u8; ADL_MAX_PATH],
    bus_number: i32,
    device_number: i32,
    function_number: i32,
    vendor_id: i32,
    adapter_name: [u8; ADL_MAX_PATH],
    display_name: [u8; ADL_MAX_PATH],
    present: i32,
    exist: i32,
    driver_path: [u8; ADL_MAX_PATH],
    driver_path_ext: [u8; ADL_MAX_PATH],
    pnp_string: [u8; ADL_MAX_PATH],
    os_display_index: i32,
}

#[repr(C)]
pub struct PmLogOutput {
    size: i32,
    sensors: [[i32; 2]; PMLOG_MAX], // [supported, value]
}

// ADL_PMLOG_SENSORS indices (adl_defines.h)
pub const PMLOG_CLK_GFXCLK: usize = 1;
pub const PMLOG_CLK_MEMCLK: usize = 2;
pub const PMLOG_TEMPERATURE_EDGE: usize = 8;
pub const PMLOG_TEMPERATURE_MEM: usize = 9;
pub const PMLOG_FAN_RPM: usize = 14;
pub const PMLOG_FAN_PERCENTAGE: usize = 15;
pub const PMLOG_INFO_ACTIVITY_GFX: usize = 19;
pub const PMLOG_GFX_VOLTAGE: usize = 21;
pub const PMLOG_ASIC_POWER: usize = 23;
pub const PMLOG_TEMPERATURE_HOTSPOT: usize = 27;
pub const PMLOG_BOARD_POWER: usize = 73;

type Malloc = unsafe extern "system" fn(i32) -> *mut c_void;
type FnCreate = unsafe extern "system" fn(Malloc, i32, *mut *mut c_void) -> i32;
type FnCtxInt = unsafe extern "system" fn(*mut c_void, *mut i32) -> i32;
type FnInfo = unsafe extern "system" fn(*mut c_void, *mut AdapterInfo, i32) -> i32;
type FnIdxInt = unsafe extern "system" fn(*mut c_void, i32, *mut i32) -> i32;
type FnPmLog = unsafe extern "system" fn(*mut c_void, i32, *mut PmLogOutput) -> i32;
type FnDestroy = unsafe extern "system" fn(*mut c_void) -> i32;

unsafe extern "system" fn adl_malloc(size: i32) -> *mut c_void {
    HeapAlloc(GetProcessHeap(), 0, size.max(0) as usize)
}

pub struct Adl {
    ctx: *mut c_void,
    adapter: i32,
    name: String,
    pmlog: FnPmLog,
    vram: Option<FnIdxInt>,
    destroy: FnDestroy,
    buf: Box<PmLogOutput>,
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

impl Adl {
    pub fn new() -> Option<Self> {
        unsafe {
            let dll: Vec<u16> = "atiadlxx.dll\0".encode_utf16().collect();
            let lib = LoadLibraryW(dll.as_ptr());
            if lib.is_null() {
                return None;
            }
            macro_rules! sym {
                ($name:literal, $t:ty) => {
                    GetProcAddress(lib, concat!($name, "\0").as_ptr())
                        .map(|f| std::mem::transmute::<_, $t>(f))
                };
            }
            let create = sym!("ADL2_Main_Control_Create", FnCreate)?;
            let destroy = sym!("ADL2_Main_Control_Destroy", FnDestroy)?;
            let num = sym!("ADL2_Adapter_NumberOfAdapters_Get", FnCtxInt)?;
            let info = sym!("ADL2_Adapter_AdapterInfo_Get", FnInfo)?;
            let pmlog = sym!("ADL2_New_QueryPMLogData_Get", FnPmLog)?;
            let vram = sym!("ADL2_Adapter_DedicatedVRAMUsage_Get", FnIdxInt);

            let mut ctx: *mut c_void = std::ptr::null_mut();
            if create(adl_malloc, 1, &mut ctx) != 0 || ctx.is_null() {
                return None;
            }
            let mut n = 0;
            if num(ctx, &mut n) != 0 || n <= 0 {
                destroy(ctx);
                return None;
            }
            let mut infos: Vec<AdapterInfo> = (0..n).map(|_| std::mem::zeroed()).collect();
            if info(
                ctx,
                infos.as_mut_ptr(),
                n * std::mem::size_of::<AdapterInfo>() as i32,
            ) != 0
            {
                destroy(ctx);
                return None;
            }
            // ADL lists one entry per display output; pick the first physical
            // AMD GPU (unique bus) that answers PM-log queries. Prefer discrete
            // cards over an APU's integrated "Radeon(TM) Graphics".
            let mut buf: Box<PmLogOutput> = Box::new(std::mem::zeroed());
            let mut candidates: Vec<(i32, String, i32)> = Vec::new(); // (index, name, bus)
            for a in &infos {
                if a.vendor_id != 1002 || candidates.iter().any(|c| c.2 == a.bus_number) {
                    continue;
                }
                buf.size = std::mem::size_of::<PmLogOutput>() as i32;
                if pmlog(ctx, a.adapter_index, &mut *buf) == 0 {
                    candidates.push((a.adapter_index, cstr(&a.adapter_name), a.bus_number));
                }
            }
            candidates.sort_by_key(|c| c.1.contains("(TM) Graphics") as u8);
            let Some((adapter, name, _)) = candidates.into_iter().next() else {
                destroy(ctx);
                return None;
            };
            Some(Self {
                ctx,
                adapter,
                name,
                pmlog,
                vram,
                destroy,
                buf,
            })
        }
    }

    /// All supported raw PM-log sensors as (index, value). Used by the probe.
    pub fn raw(&mut self) -> Vec<(usize, i32)> {
        if !self.query() {
            return Vec::new();
        }
        (0..PMLOG_MAX)
            .filter(|&i| self.buf.sensors[i][0] != 0)
            .map(|i| (i, self.buf.sensors[i][1]))
            .collect()
    }

    fn query(&mut self) -> bool {
        self.buf.size = std::mem::size_of::<PmLogOutput>() as i32;
        unsafe { (self.pmlog)(self.ctx, self.adapter, &mut *self.buf) == 0 }
    }

    fn get(&self, i: usize) -> Option<f64> {
        let s = self.buf.sensors[i];
        (s[0] != 0).then_some(s[1] as f64)
    }
}

impl GpuSource for Adl {
    fn name(&self) -> String {
        format!("{} (AMD ADL)", self.name)
    }

    fn poll(&mut self, s: &mut Snapshot) {
        if self.query() {
            s.set(SensorId::GpuLoad, self.get(PMLOG_INFO_ACTIVITY_GFX));
            s.set(SensorId::GpuClock, self.get(PMLOG_CLK_GFXCLK));
            s.set(SensorId::GpuMemClock, self.get(PMLOG_CLK_MEMCLK));
            s.set(SensorId::GpuTemp, self.get(PMLOG_TEMPERATURE_EDGE));
            s.set(SensorId::GpuHotspot, self.get(PMLOG_TEMPERATURE_HOTSPOT));
            s.set(SensorId::GpuMemTemp, self.get(PMLOG_TEMPERATURE_MEM));
            s.set(SensorId::GpuFan, self.get(PMLOG_FAN_RPM));
            s.set(SensorId::GpuFanPct, self.get(PMLOG_FAN_PERCENTAGE));
            s.set(SensorId::GpuVoltage, self.get(PMLOG_GFX_VOLTAGE));
            s.set(
                SensorId::GpuPower,
                self.get(PMLOG_BOARD_POWER).or(self.get(PMLOG_ASIC_POWER)),
            );
        }
        if let Some(f) = self.vram {
            let mut mb = 0;
            if unsafe { f(self.ctx, self.adapter, &mut mb) } == 0 {
                s.set(SensorId::GpuVram, Some(mb as f64));
            }
        }
    }
}

impl Drop for Adl {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.ctx) };
    }
}
