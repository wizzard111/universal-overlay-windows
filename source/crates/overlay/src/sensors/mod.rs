//! Hardware sensors. Each source fills the values it knows into a
//! [`Snapshot`]; values nobody provides stay `None` and are drawn as "—".
//!
//! Every OS backend exposes `Sensors::new()`, `poll()` and `gpu_name()`.

pub mod nvml;

pub mod windows;
pub use windows::*;

use uo_common::sensor::SensorId;

/// Which groups of sources a tick has to poll. Built from the config:
/// sensors that are neither shown nor graphed are not read at all (no
/// driver calls, the CPU-temperature service is not woken).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Want {
    pub cpu: bool,
    pub cpu_temp: bool,
    pub ram: bool,
    pub gpu: bool,
}

impl Want {
    pub const ALL: Want = Want {
        cpu: true,
        cpu_temp: true,
        ram: true,
        gpu: true,
    };

    pub fn from_ids(ids: impl Iterator<Item = SensorId>) -> Self {
        let mut w = Want::default();
        for id in ids {
            match id {
                SensorId::CpuLoad | SensorId::CpuClock => w.cpu = true,
                SensorId::CpuTemp => w.cpu_temp = true,
                SensorId::RamUsed | SensorId::RamLoad => w.ram = true,
                SensorId::GpuLoad
                | SensorId::GpuTemp
                | SensorId::GpuHotspot
                | SensorId::GpuMemTemp
                | SensorId::GpuClock
                | SensorId::GpuMemClock
                | SensorId::GpuPower
                | SensorId::GpuVoltage
                | SensorId::GpuFan
                | SensorId::GpuFanPct
                | SensorId::GpuVram => w.gpu = true,
                _ => {}
            }
        }
        w
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    v: [Option<f64>; SensorId::COUNT],
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            v: [None; SensorId::COUNT],
        }
    }
}

impl Snapshot {
    pub fn get(&self, id: SensorId) -> Option<f64> {
        self.v[id.index()]
    }

    pub fn set(&mut self, id: SensorId, v: Option<f64>) {
        self.v[id.index()] = v.filter(|x| x.is_finite());
    }
}

pub trait GpuSource {
    fn name(&self) -> String;
    fn poll(&mut self, s: &mut Snapshot);
}
