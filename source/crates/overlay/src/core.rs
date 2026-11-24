//! Platform-independent overlay logic. A platform host owns the window,
//! tray, hotkey and timers and calls into [`Core`]:
//!   * `tick(target_pid)` every `update_interval_ms` (sensors + FPS text),
//!   * `render(monitor, dpi)` on every redraw (also graph timer),
//!     then presents `core.canvas` at the returned position.

use crate::frames::{self, Status};
use crate::render::{Canvas, PxRect, Text};
use crate::sensors::{Sensors, Snapshot, Want};
use crate::view::{self, GraphRect, ViewInput};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use uo_common::config::{Config, GraphMode};
use uo_common::sensor::SensorId;

/// Sampled history of one sensor: (seconds since start, value).
pub type SensorHistory = HashMap<SensorId, VecDeque<(f32, f32)>>;

/// A monitor rectangle in the coordinate system the platform positions
/// windows in (pixels on Windows/X11, points on macOS).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub struct Core {
    pub cfg: Config,
    pub cfg_error: Option<String>,
    pub frame_status: Status,
    pub canvas: Canvas,
    sensors: Sensors,
    snap: Snapshot,
    session: Option<frames::Session>,
    text: Text,
    freq: f64,
    /// Everything but the graph, redrawn only when the numbers change.
    base: Canvas,
    graph_rect: Option<GraphRect>,
    base_key: Option<(Rect, u32)>,
    /// History of the sensors that have a graph (only those).
    hist: SensorHistory,
    t0: Instant,
    /// `canvas` holds the full current picture (base + graph); when false
    /// the next render copies the whole base layer.
    canvas_valid: bool,
    /// What changed in `canvas` during the last render: `None` = all of it,
    /// otherwise only this rectangle (the graph). Hosts that can update a
    /// part of their window use it.
    pub dirty: Option<PxRect>,
}

/// Font fallbacks per OS, tried after the configured font.
const FALLBACK_FONTS: &[&str] = &["Segoe UI Semibold", "Segoe UI", "Arial", "Tahoma"];

fn try_font(path: &std::path::Path) -> Option<Text> {
    let t = Text::new(Some(std::fs::read(path).ok()?));
    t.has_font().then_some(t)
}

pub fn load_font(name: &str) -> Text {
    for n in std::iter::once(name).chain(FALLBACK_FONTS.iter().copied()) {
        if let Some(t) = uo_common::fonts::resolve(n).and_then(|p| try_font(&p)) {
            return t;
        }
    }
    // Last resort (e.g. Linux without fontconfig, names are file stems):
    // the best-looking installed sans font, bold preferred.
    let score = |n: &str| -> i32 {
        let l = n.to_lowercase().replace(['-', '_', ' '], "");
        let mut s = 0;
        if l.contains("sans") {
            s += 4;
        }
        if l.contains("bold") || l.contains("semibold") {
            s += 2;
        }
        for bad in [
            "mono",
            "serif",
            "italic",
            "oblique",
            "condensed",
            "math",
            "symbol",
            "emoji",
            "awesome",
        ] {
            if l.contains(bad) && !(bad == "serif" && l.contains("sansserif")) {
                s -= 6;
            }
        }
        if [
            "dejavu",
            "noto",
            "liberation",
            "ubuntu",
            "cantarell",
            "lato",
            "inter",
            "roboto",
        ]
        .iter()
        .any(|k| l.contains(k))
        {
            s += 3;
        }
        s
    };
    let mut fonts = uo_common::fonts::list_fonts();
    fonts.sort_by_key(|f| -score(&f.name));
    fonts
        .iter()
        .find_map(|f| try_font(&f.path))
        .unwrap_or_else(|| Text::new(None))
}

pub fn uses_frames(cfg: &Config) -> bool {
    cfg.graph.enabled
        || cfg.sensors.iter().any(|s| {
            s.enabled
                && matches!(
                    s.id,
                    SensorId::Fps | SensorId::FrameTime | SensorId::Low1 | SensorId::Low01
                )
        })
}

/// Local wall-clock time as "HH:MM:SS".
pub fn local_hms() -> String {
    {
        let mut t = unsafe { std::mem::zeroed() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
        format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
    }
}

impl Core {
    pub fn new() -> Self {
        let (cfg, cfg_error) = Config::load();
        let text = load_font(&cfg.style.font);
        Self {
            cfg,
            cfg_error,
            frame_status: Status::Off,
            canvas: Canvas::new(),
            sensors: Sensors::new(),
            snap: Snapshot::default(),
            session: None,
            text,
            freq: frames::freq(),
            base: Canvas::new(),
            graph_rect: None,
            base_key: None,
            hist: HashMap::new(),
            t0: Instant::now(),
            canvas_valid: false,
            dirty: None,
        }
    }

    pub fn gpu_name(&self) -> String {
        self.sensors.gpu_name()
    }

    /// Re-read the config. Returns true if the hotkey changed.
    pub fn reload(&mut self, visible: bool) -> bool {
        let (cfg, err) = Config::load();
        let font_changed = cfg.style.font != self.cfg.style.font;
        let hotkey_changed = cfg.general.hotkey != self.cfg.general.hotkey;
        self.base_key = None;
        self.cfg = cfg;
        self.cfg_error = err;
        if font_changed {
            self.text = load_font(&self.cfg.style.font);
        }
        frames::set_history_seconds(self.cfg.history_seconds());
        if visible {
            if uses_frames(&self.cfg) {
                self.start_frames();
            } else {
                self.stop_frames();
            }
        }
        hotkey_changed
    }

    pub fn start_frames(&mut self) {
        if self.session.is_some() || !uses_frames(&self.cfg) {
            return;
        }
        match frames::Session::start(self.cfg.history_seconds()) {
            Ok(s) => {
                self.session = Some(s);
                self.frame_status = Status::Running;
            }
            Err(e) => self.frame_status = e,
        }
    }

    pub fn stop_frames(&mut self) {
        self.session = None;
        if self.frame_status == Status::Running {
            self.frame_status = Status::Off;
        }
    }

    /// Called when the overlay is hidden: drop everything that costs memory.
    pub fn on_hidden(&mut self) {
        self.stop_frames();
        self.canvas = Canvas::new();
        self.base = Canvas::new();
        self.base_key = None;
        self.canvas_valid = false;
        self.hist = HashMap::new();
    }

    /// Poll sensors and recompute frame statistics.
    pub fn tick(&mut self, target_pid: Option<u32>) {
        if let Some(pid) = target_pid {
            frames::set_target(pid);
        }
        let cfg = &self.cfg;
        let shown = cfg.sensors.iter().filter(|s| s.enabled).map(|s| s.id);
        let graphed = cfg
            .sensor_graphs
            .iter()
            .filter(|g| g.enabled)
            .map(|g| g.sensor);
        self.sensors
            .poll(&mut self.snap, Want::from_ids(shown.chain(graphed)));
        let st = &self.cfg.stats;
        let need =
            frames::Tail::secs((st.fps_window_ms as f64 / 1000.0).max(st.lows_window_s as f64));
        let fs = frames::snapshot(need)
            .map(|f| frames::compute_stats(&f, self.freq, &self.cfg.stats))
            .unwrap_or_default();
        self.snap.set(SensorId::Fps, fs.fps);
        self.snap.set(SensorId::FrameTime, fs.frame_time);
        self.snap.set(SensorId::Low1, fs.low1);
        self.snap.set(SensorId::Low01, fs.low01);
        self.record_history();
        self.base_key = None; // numbers changed: rebuild the base layer
    }

    /// Append the current values of graphed sensors and trim old samples.
    fn record_history(&mut self) {
        let now = self.t0.elapsed().as_secs_f32();
        let graphs = &self.cfg.sensor_graphs;
        self.hist
            .retain(|id, _| graphs.iter().any(|g| g.enabled && g.sensor == *id));
        for g in graphs.iter().filter(|g| g.enabled) {
            let keep = graphs
                .iter()
                .filter(|o| o.enabled && o.sensor == g.sensor)
                .map(|o| o.window_s)
                .fold(0.0, f32::max)
                + 1.0;
            let h = self.hist.entry(g.sensor).or_default();
            if h.back().is_none_or(|&(t, _)| t < now) {
                if let Some(v) = self.snap.get(g.sensor) {
                    h.push_back((now, v as f32));
                }
            }
            while h.front().is_some_and(|&(t, _)| now - t > keep) {
                h.pop_front();
            }
        }
    }

    fn note(&self) -> Option<&'static str> {
        if self.cfg_error.is_some() {
            return Some("Error in config.toml — using default settings");
        }
        if !uses_frames(&self.cfg) {
            return None;
        }
        match self.frame_status {
            Status::AccessDenied => {
                Some("FPS: sign out of Windows and back in (frame counters access)")
            }
            Status::NoLayer => Some("FPS: install the Vulkan layer (tray menu)"),
            Status::Unsupported => Some("FPS is not available on this OS"),
            Status::NoPermission => Some("FPS: allow Screen Recording (System Settings → Privacy)"),
            Status::Error(_) => Some("FPS: could not start frame capture"),
            _ => None,
        }
    }

    /// Draw into `self.canvas` for a monitor with the given DPI scale and
    /// return the top-left position (in the monitor's coordinate system,
    /// where `px_per_unit` canvas pixels make one unit).
    pub fn render(&mut self, mon: Rect, dpi: f32, px_per_unit: f32) -> (i32, i32) {
        let scale = dpi * self.cfg.general.scale;
        let gr = &self.cfg.graph;
        let frames_vec = if gr.enabled {
            let need = match gr.mode {
                GraphMode::Time => frames::Tail::secs(gr.time_window_s as f64 + 0.5),
                GraphMode::Frames => frames::Tail {
                    secs: 0.0,
                    min_count: gr.frame_count as usize + 1,
                },
            };
            frames::snapshot(need).unwrap_or_default()
        } else {
            Vec::new()
        };
        let clock = if self
            .cfg
            .sensors
            .iter()
            .any(|s| s.enabled && s.id == SensorId::Clock)
        {
            local_hms()
        } else {
            String::new()
        };
        let note = self.note();
        let inp = ViewInput {
            cfg: &self.cfg,
            snap: &self.snap,
            frames: &frames_vec,
            qpf: self.freq,
            now_qpc: frames::now(),
            scale,
            clock: &clock,
            note,
            hist: &self.hist,
            now_s: self.t0.elapsed().as_secs_f32(),
        };
        let key = (mon, dpi.to_bits());
        if self.base_key != Some(key) {
            self.graph_rect = view::draw_base(&mut self.text, &mut self.base, &inp);
            self.base_key = Some(key);
            self.canvas_valid = false;
        }
        let op = self.cfg.general.opacity;
        // Between sensor ticks only the frame-time graph changes: restore and
        // redraw just its rectangle instead of the whole overlay.
        let graph_px = self.graph_rect.map(|r| {
            let x0 = r.x.floor().max(0.0) as i32;
            let y0 = r.y.floor().max(0.0) as i32;
            let x1 = ((r.x + r.w).ceil() as i32).min(self.base.w);
            let y1 = ((r.y + r.stats_h + r.h).ceil() as i32).min(self.base.h);
            PxRect {
                x: x0,
                y: y0,
                w: (x1 - x0).max(0),
                h: (y1 - y0).max(0),
            }
        });
        match graph_px {
            Some(px)
                if self.canvas_valid
                    && self.canvas.w == self.base.w
                    && self.canvas.h == self.base.h =>
            {
                self.canvas.copy_rect_from(&self.base, px);
                if let Some(r) = self.graph_rect {
                    view::draw_graph(&mut self.text, &mut self.canvas, &inp, r);
                }
                if op < 0.999 {
                    self.canvas.scale_alpha_rect(op, px);
                }
                self.dirty = Some(px);
            }
            _ => {
                self.canvas.copy_from(&self.base);
                if let Some(r) = self.graph_rect {
                    view::draw_graph(&mut self.text, &mut self.canvas, &inp, r);
                }
                if op < 0.999 {
                    self.canvas.scale_alpha(op);
                }
                self.canvas_valid = true;
                self.dirty = None;
            }
        }

        let g = &self.cfg.general;
        let w = (self.canvas.w as f32 / px_per_unit).round() as i32;
        let h = (self.canvas.h as f32 / px_per_unit).round() as i32;
        let unit_dpi = dpi / px_per_unit;
        let ox = (g.offset_x as f32 * unit_dpi) as i32;
        let oy = (g.offset_y as f32 * unit_dpi) as i32;
        let (gx, gy) = g.anchor.grid();
        let x = match gx {
            0 => mon.x + ox,
            1 => mon.x + (mon.w - w) / 2 + ox,
            _ => mon.x + mon.w - w - ox,
        };
        let y = match gy {
            0 => mon.y + oy,
            1 => mon.y + (mon.h - h) / 2 + oy,
            _ => mon.y + mon.h - h - oy,
        };
        (x, y)
    }
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

/// Small tray/menu-bar icon as straight-alpha ARGB pixels (0xAARRGGBB).
pub fn icon_pixels(size: i32) -> Vec<u32> {
    use uo_common::config::Color;
    let mut cv = Canvas::new();
    cv.resize_clear(size, size);
    let s = size as f32;
    cv.fill_round_rect(
        0.0,
        0.0,
        s,
        s,
        s * 0.22,
        Color::rgba(0x12, 0x12, 0x12, 0xFF),
    );
    let pts = [0.62, 0.58, 0.64, 0.60, 0.22, 0.60, 0.57, 0.62, 0.59];
    let line = Color::rgba(0xEC, 0xEC, 0xEC, 0xFF);
    let spike = Color::rgba(0xB8, 0x24, 0x2D, 0xFF);
    let x0 = s * 0.14;
    let w = s * 0.72;
    for x in 0..size {
        let fx = (x as f32 - x0) / w;
        if !(0.0..=1.0).contains(&fx) {
            continue;
        }
        let p = fx * (pts.len() - 1) as f32;
        let i = (p as usize).min(pts.len() - 2);
        let t = p - i as f32;
        let v = pts[i] + (pts[i + 1] - pts[i]) * t;
        let y = v * s;
        cv.vspan(
            x,
            y - s * 0.055,
            y + s * 0.055,
            if v < 0.45 { spike } else { line },
        );
    }
    cv.px
        .iter()
        .map(|&p| {
            let a = p >> 24;
            if a == 0 {
                return 0;
            }
            let un = |c: u32| ((c * 255 + a / 2) / a).min(255);
            (a << 24) | (un((p >> 16) & 255) << 16) | (un((p >> 8) & 255) << 8) | un(p & 255)
        })
        .collect()
}
