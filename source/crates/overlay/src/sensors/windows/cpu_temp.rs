//! CPU temperature from the `uo-sensord` service (which owns the PawnIO
//! driver). The overlay stays a normal, non-elevated program: it only
//! signals "I want a reading" and reads the result from shared memory.
//! See `uo_common::sensord` for the protocol.

use super::super::Snapshot;
use std::sync::atomic::Ordering;
use uo_common::sensor::SensorId;
use uo_common::sensord::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Threading::*;

pub struct CpuTemp {
    map: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    want: HANDLE,
    done: HANDLE,
}

/// Why the CPU temperature is unavailable (shown in the probe).
#[derive(Debug)]
pub enum Unavailable {
    /// The service is not installed or not running.
    NoService,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

impl CpuTemp {
    pub fn new() -> Result<Self, Unavailable> {
        unsafe {
            let map = OpenFileMappingW(FILE_MAP_READ, 0, wide(MAPPING).as_ptr());
            if map.is_null() {
                return Err(Unavailable::NoService);
            }
            let view = MapViewOfFile(map, FILE_MAP_READ, 0, 0, SIZE);
            let access = EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE;
            let want = OpenEventW(access, 0, wide(EVENT_WANT).as_ptr());
            let done = OpenEventW(access, 0, wide(EVENT_DONE).as_ptr());
            let t = CpuTemp {
                map,
                view,
                want,
                done,
            };
            if view.Value.is_null()
                || want.is_null()
                || done.is_null()
                || t.shared().magic.load(Ordering::Acquire) != MAGIC
            {
                return Err(Unavailable::NoService);
            }
            Ok(t)
        }
    }

    fn shared(&self) -> &Shared {
        unsafe { &*(self.view.Value as *const Shared) }
    }

    /// Service-side status (driver / CPU support).
    pub fn status(&self) -> i32 {
        self.shared().status.load(Ordering::Relaxed)
    }

    /// Ask for a fresh reading and wait for it (a few ms at most).
    pub fn poll(&self, s: &mut Snapshot) {
        unsafe {
            ResetEvent(self.done);
            SetEvent(self.want);
            WaitForSingleObject(self.done, 15);
        }
        let sh = self.shared();
        let v = f32::from_bits(sh.cpu_temp.load(Ordering::Relaxed));
        let ok = sh.status.load(Ordering::Relaxed) == STATUS_OK && v.is_finite();
        s.set(SensorId::CpuTemp, ok.then_some(v as f64));
    }
}

impl Drop for CpuTemp {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.view);
            if !self.want.is_null() {
                CloseHandle(self.want);
            }
            if !self.done.is_null() {
                CloseHandle(self.done);
            }
            CloseHandle(self.map);
        }
    }
}
