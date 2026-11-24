//! Vendor-neutral fallback (Intel, or anything without a vendor API):
//! Windows "GPU Engine" / "GPU Adapter Memory" counters — the same data
//! Task Manager shows. Gives load and VRAM, no temperatures/clocks.

use super::pdh::{Counter, Query};
use super::{GpuSource, Snapshot};
use std::collections::HashMap;
use uo_common::sensor::SensorId;

pub struct PdhGpu {
    q: Query,
    engine: Counter,
    mem: Option<Counter>,
}

/// "pid_1_luid_0x0_0xD1B5_phys_0_eng_3_engtype_3D" -> "luid_0x0_0xD1B5_phys_0"
fn adapter_key(instance: &str) -> &str {
    let start = instance.find("luid").unwrap_or(0);
    let end = instance.find("_eng").unwrap_or(instance.len());
    &instance[start..end.max(start)]
}

impl PdhGpu {
    pub fn new() -> Option<Self> {
        let q = Query::new()?;
        let engine = q.add("\\GPU Engine(*engtype_3D)\\Utilization Percentage")?;
        let mem = q.add("\\GPU Adapter Memory(*)\\Dedicated Usage");
        q.collect();
        Some(Self { q, engine, mem })
    }
}

impl GpuSource for PdhGpu {
    fn name(&self) -> String {
        "Windows GPU counters".into()
    }

    fn poll(&mut self, s: &mut Snapshot) {
        if !self.q.collect() {
            return;
        }
        let mut per_adapter: HashMap<String, f64> = HashMap::new();
        for (inst, v) in self.q.array(self.engine) {
            *per_adapter
                .entry(adapter_key(&inst).to_string())
                .or_default() += v;
        }
        let load = per_adapter
            .values()
            .copied()
            .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
        s.set(SensorId::GpuLoad, load.map(|v| v.min(100.0)));
        if let Some(c) = self.mem {
            let max = self
                .q
                .array(c)
                .into_iter()
                .map(|(_, v)| v)
                .fold(0.0, f64::max);
            if max > 0.0 {
                s.set(SensorId::GpuVram, Some(max / (1024.0 * 1024.0)));
            }
        }
    }
}
