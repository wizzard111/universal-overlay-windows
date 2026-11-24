//! Frame timing. Each OS has a backend that collects present timestamps
//! of the target process; statistics are computed here, identically for
//! every backend.
//!
//! Backend API (same names on every OS):
//!   `Session::start(history_s)`, `set_target(pid)`, `set_history_seconds(s)`,
//!   `snapshot(Tail) -> Option<Vec<Frame>>`, `now()`, `freq()`, `LATENCY_S`.

use uo_common::config::{LowsMethod, Stats};

mod etw;
pub use etw::*;

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// Timestamp of the present call in backend ticks (see `freq()`).
    pub ts: i64,
    /// Time since the previous present of the same swap chain.
    pub ms: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Off,
    Running,
    /// Windows: the ETW session needs Administrator rights.
    AccessDenied,
    /// Linux: the Vulkan layer is not installed.
    NoLayer,
    /// The OS gives no way to measure frames without injection.
    Unsupported,
    /// macOS: the "Screen Recording" permission is missing.
    NoPermission,
    Error(u32),
}

/// Which part of the frame history a caller needs: everything newer than
/// `secs` before the last frame, but at least the last `min_count` frames.
/// Copying only this tail keeps the per-redraw cost independent of the
/// history length.
#[derive(Clone, Copy, Debug)]
pub struct Tail {
    pub secs: f64,
    pub min_count: usize,
}

impl Tail {
    pub const fn secs(secs: f64) -> Self {
        Self { secs, min_count: 0 }
    }
}

pub(crate) fn tail(frames: &std::collections::VecDeque<Frame>, freq: f64, t: Tail) -> Vec<Frame> {
    let Some(last) = frames.back() else {
        return Vec::new();
    };
    let from = last.ts - (t.secs * freq) as i64;
    let by_time = frames.partition_point(|f| f.ts < from);
    let start = by_time.min(frames.len().saturating_sub(t.min_count));
    frames.range(start..).copied().collect()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub fps: Option<f64>,
    pub frame_time: Option<f64>,
    pub low1: Option<f64>,
    pub low01: Option<f64>,
}

pub fn compute_stats(frames: &[Frame], freq: f64, cfg: &Stats) -> FrameStats {
    let mut out = FrameStats::default();
    let Some(last) = frames.last() else {
        return out;
    };
    let fps_from = last.ts - (cfg.fps_window_ms as f64 / 1000.0 * freq) as i64;
    let (mut n, mut sum) = (0u32, 0f64);
    for f in frames.iter().rev().take_while(|f| f.ts > fps_from) {
        n += 1;
        sum += f.ms as f64;
    }
    if n > 0 && sum > 0.0 {
        out.fps = Some(n as f64 * 1000.0 / sum);
        out.frame_time = Some(sum / n as f64);
    }
    let lows_from = last.ts - (cfg.lows_window_s as f64 * freq) as i64;
    let mut v: Vec<f32> = frames
        .iter()
        .rev()
        .take_while(|f| f.ts > lows_from)
        .map(|f| f.ms)
        .collect();
    if v.len() >= 10 {
        // O(n) selection instead of a full sort: at high frame rates the
        // lows window holds tens of thousands of frames.
        let desc = |a: &f32, b: &f32| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal);
        let mut low = |frac: f64| -> Option<f64> {
            let k = ((v.len() as f64 * frac).ceil() as usize).clamp(1, v.len());
            let (worst, kth, _) = v.select_nth_unstable_by(k - 1, desc);
            let ms = match cfg.lows_method {
                LowsMethod::Percentile => *kth as f64,
                LowsMethod::Average => {
                    (worst.iter().map(|&x| x as f64).sum::<f64>() + *kth as f64) / k as f64
                }
            };
            (ms > 0.0).then(|| 1000.0 / ms)
        };
        out.low1 = low(0.01);
        out.low01 = low(0.001);
    }
    out
}

/// Shared helper for backends: frames of one swap chain plus bookkeeping.
pub(crate) struct Stream {
    pub prio: u8,
    pub last_ts: i64,
    pub frames: std::collections::VecDeque<Frame>,
}

impl Stream {
    pub fn new(prio: u8) -> Self {
        Self {
            prio,
            last_ts: 0,
            frames: std::collections::VecDeque::with_capacity(1024),
        }
    }

    /// Append a present timestamp; `keep` is the history length in ticks.
    pub fn push(&mut self, ts: i64, freq: f64, keep: i64) {
        if self.last_ts > 0 && ts > self.last_ts {
            let ms = ((ts - self.last_ts) as f64 * 1000.0 / freq) as f32;
            if ms < 10_000.0 {
                self.frames.push_back(Frame { ts, ms });
            }
        }
        self.last_ts = self.last_ts.max(ts);
        while let Some(f) = self.frames.front() {
            if ts - f.ts > keep || self.frames.len() > 50_000 {
                self.frames.pop_front();
            } else {
                break;
            }
        }
    }
}

/// Pick the stream that represents "the game": highest priority among the
/// streams that presented recently, then the busiest in the last second.
pub(crate) fn best_stream<'a>(
    streams: impl Iterator<Item = &'a Stream>,
    now: i64,
    freq: f64,
) -> Option<&'a Stream> {
    let recent = now - (1.5 * freq) as i64;
    let one_s = now - freq as i64;
    streams
        .filter(|s| s.last_ts >= recent && !s.frames.is_empty())
        .max_by_key(|s| {
            let n = s.frames.iter().rev().take_while(|f| f.ts >= one_s).count();
            (s.prio, n)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_constant_rate() {
        let freq: f64 = 10_000_000.0;
        let frames: Vec<Frame> = (1..=600)
            .map(|i| Frame {
                ts: (i as f64 * 0.01 * freq) as i64,
                ms: 10.0,
            })
            .collect();
        let s = compute_stats(&frames, freq, &Stats::default());
        assert!((s.fps.unwrap() - 100.0).abs() < 1e-6);
        assert!((s.low1.unwrap() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn lows_catch_spikes() {
        let freq: f64 = 10_000_000.0;
        let mut t = 0.0f64;
        let frames: Vec<Frame> = (0..1000)
            .map(|i| {
                let ms: f32 = if i % 50 == 0 { 50.0 } else { 10.0 };
                t += ms as f64 / 1000.0;
                Frame {
                    ts: (t * freq) as i64,
                    ms,
                }
            })
            .collect();
        let s = compute_stats(&frames, freq, &Stats::default());
        assert!((s.low1.unwrap() - 20.0).abs() < 1e-6, "{:?}", s);
    }

    #[test]
    fn stream_push_and_trim() {
        let mut s = Stream::new(1);
        for i in 0..100 {
            s.push(i * 10, 1000.0, 500);
        }
        assert!(s.frames.len() <= 51);
        assert!((s.frames.back().unwrap().ms - 10.0).abs() < 1e-6);
    }

    #[test]
    fn tail_by_time_and_count() {
        let d: std::collections::VecDeque<Frame> = (0..100)
            .map(|i| Frame {
                ts: i * 10,
                ms: 10.0,
            })
            .collect();
        assert_eq!(tail(&d, 1000.0, Tail::secs(0.1)).len(), 11);
        assert_eq!(
            tail(
                &d,
                1000.0,
                Tail {
                    secs: 0.0,
                    min_count: 30
                }
            )
            .len(),
            30
        );
    }

    #[test]
    fn lows_average_method() {
        let freq = 1000.0;
        let mut t = 0.0;
        let frames: Vec<Frame> = (0..1000)
            .map(|i| {
                let ms: f32 = if i < 10 { 40.0 } else { 10.0 };
                t += ms as f64;
                Frame { ts: t as i64, ms }
            })
            .collect();
        let cfg = Stats {
            lows_method: LowsMethod::Average,
            lows_window_s: 100.0,
            ..Stats::default()
        };
        let s = compute_stats(&frames, freq, &cfg);
        assert!((s.low1.unwrap() - 25.0).abs() < 1e-6, "{s:?}");
    }
}
