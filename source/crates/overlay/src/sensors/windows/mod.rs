//! Windows sensors: PDH for CPU, ADL / NVML / GPU counters for the GPU,
//! the uo-sensord service for the CPU temperature.

pub mod cpu;
pub mod cpu_temp;
pub mod gpu_amd;
pub mod gpu_pdh;
pub(crate) mod pdh;

use super::{nvml, GpuSource, Snapshot, Want};
use std::time::{Duration, Instant};

pub struct Sensors {
    cpu: cpu::Cpu,
    gpu: Option<Box<dyn GpuSource>>,
    cpu_temp: Result<cpu_temp::CpuTemp, cpu_temp::Unavailable>,
    /// Last attempt to connect to the service (it may start after us).
    temp_retry: Instant,
}

impl Default for Sensors {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensors {
    pub fn new() -> Self {
        let gpu: Option<Box<dyn GpuSource>> = if let Some(a) = gpu_amd::Adl::new() {
            Some(Box::new(a))
        } else if let Some(n) = nvml::Nvml::new() {
            Some(Box::new(n))
        } else if let Some(p) = gpu_pdh::PdhGpu::new() {
            Some(Box::new(p))
        } else {
            None
        };
        Self {
            cpu: cpu::Cpu::new(),
            gpu,
            cpu_temp: cpu_temp::CpuTemp::new(),
            temp_retry: Instant::now(),
        }
    }

    /// Human-readable state of the CPU temperature source.
    pub fn cpu_temp_status(&self) -> String {
        match &self.cpu_temp {
            Ok(t) => format!(
                "uo-sensord service: {}",
                uo_common::sensord::status_text(t.status())
            ),
            Err(_) => "uo-sensord service is not running".into(),
        }
    }

    pub fn gpu_name(&self) -> String {
        self.gpu
            .as_ref()
            .map(|g| g.name())
            .unwrap_or_else(|| "none".into())
    }

    pub fn poll(&mut self, s: &mut Snapshot, want: Want) {
        if want.cpu || want.ram {
            self.cpu.poll(s);
        }
        if !want.cpu_temp {
            // nothing to read: the service stays asleep
        } else if self.cpu_temp.is_err() && self.temp_retry.elapsed() > Duration::from_secs(5) {
            self.cpu_temp = cpu_temp::CpuTemp::new();
            self.temp_retry = Instant::now();
        }
        if want.cpu_temp {
            if let Ok(t) = &self.cpu_temp {
                t.poll(s);
            }
        }
        if want.gpu {
            if let Some(g) = self.gpu.as_mut() {
                g.poll(s);
            }
        }
    }
}
