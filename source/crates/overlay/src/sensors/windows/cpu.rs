//! CPU load, CPU effective clock and RAM.
//!
//! * Load = "% Processor Utility" — the frequency-aware counter Task Manager
//!   uses since Windows 8 (capped to 100). Fallback: GetSystemTimes.
//! * Clock = nominal frequency × "% Processor Performance" / 100, which is
//!   exactly how Task Manager computes "Speed" (includes boost).
//! * RAM = GlobalMemoryStatusEx.

use super::pdh::{Counter, Query};
use super::Snapshot;
use uo_common::sensor::SensorId;
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

/// Query + total load, frequency and performance counters.
type PdhCounters = (Query, Option<Counter>, Option<Counter>, Option<Counter>);

pub struct Cpu {
    pdh: Option<PdhCounters>,
    last_times: Option<(u64, u64)>,
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

impl Cpu {
    pub fn new() -> Self {
        let pdh = Query::new().map(|q| {
            let util = q.add("\\Processor Information(_Total)\\% Processor Utility");
            let perf = q.add("\\Processor Information(_Total)\\% Processor Performance");
            let freq = q.add("\\Processor Information(_Total)\\Processor Frequency");
            q.collect(); // rate counters need a first sample
            (q, util, perf, freq)
        });
        Self {
            pdh,
            last_times: None,
        }
    }

    fn system_times_load(&mut self) -> Option<f64> {
        let (mut idle, mut kernel, mut user) =
            unsafe { std::mem::zeroed::<(FILETIME, FILETIME, FILETIME)>() };
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return None;
        }
        let idle = ft(idle);
        let total = ft(kernel) + ft(user); // kernel time includes idle
        let prev = self.last_times.replace((idle, total));
        let (pi, pt) = prev?;
        let dt = total.saturating_sub(pt);
        if dt == 0 {
            return None;
        }
        Some((1.0 - idle.saturating_sub(pi) as f64 / dt as f64) * 100.0)
    }

    pub fn poll(&mut self, s: &mut Snapshot) {
        let mut load = None;
        let mut clock = None;
        if let Some((q, util, perf, freq)) = &self.pdh {
            if q.collect() {
                load = util.and_then(|c| q.value(c)).map(|v| v.min(100.0));
                if let (Some(p), Some(f)) =
                    (perf.and_then(|c| q.value(c)), freq.and_then(|c| q.value(c)))
                {
                    clock = Some(f * p / 100.0);
                }
            }
        }
        let st_load = self.system_times_load();
        s.set(
            SensorId::CpuLoad,
            load.or(st_load).map(|v| v.clamp(0.0, 100.0)),
        );
        s.set(SensorId::CpuClock, clock);

        let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut m) } != 0 && m.ullTotalPhys > 0 {
            let used = m.ullTotalPhys - m.ullAvailPhys;
            s.set(SensorId::RamUsed, Some(used as f64 / (1024.0 * 1024.0)));
            s.set(
                SensorId::RamLoad,
                Some(used as f64 * 100.0 / m.ullTotalPhys as f64),
            );
        }
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}
