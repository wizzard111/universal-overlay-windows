use crate::sensor::SensorId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::PathBuf;

// ---------------------------------------------------------------- Color

/// RGBA colour, stored in the config as "#RRGGBBAA".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        let b = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
        match s.len() {
            6 => Some(Self::rgba(b(0)?, b(2)?, b(4)?, 255)),
            8 => Some(Self::rgba(b(0)?, b(2)?, b(4)?, b(6)?)),
            _ => None,
        }
    }

    pub fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("bad color '{s}'")))
    }
}

// ---------------------------------------------------------------- enums

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Anchor {
    pub const ALL: &'static [Anchor] = &[
        Anchor::TopLeft,
        Anchor::TopCenter,
        Anchor::TopRight,
        Anchor::MiddleLeft,
        Anchor::MiddleRight,
        Anchor::BottomLeft,
        Anchor::BottomCenter,
        Anchor::BottomRight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Anchor::TopLeft => "Сверху слева",
            Anchor::TopCenter => "Сверху по центру",
            Anchor::TopRight => "Сверху справа",
            Anchor::MiddleLeft => "Слева по центру",
            Anchor::MiddleRight => "Справа по центру",
            Anchor::BottomLeft => "Снизу слева",
            Anchor::BottomCenter => "Снизу по центру",
            Anchor::BottomRight => "Снизу справа",
        }
    }

    /// (horizontal, vertical) position as 0 = start, 1 = center, 2 = end.
    pub fn grid(self) -> (u8, u8) {
        match self {
            Anchor::TopLeft => (0, 0),
            Anchor::TopCenter => (1, 0),
            Anchor::TopRight => (2, 0),
            Anchor::MiddleLeft => (0, 1),
            Anchor::MiddleRight => (2, 1),
            Anchor::BottomLeft => (0, 2),
            Anchor::BottomCenter => (1, 2),
            Anchor::BottomRight => (2, 2),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorMode {
    /// Monitor that holds the focused window (the game).
    Foreground,
    Primary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphMode {
    /// X axis is wall-clock time (`time_window_s`).
    Time,
    /// X axis is the last `frame_count` frames, evenly spaced.
    Frames,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphScale {
    Auto,
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LowsMethod {
    /// 1% low = FPS at the 99th percentile frame time.
    Percentile,
    /// 1% low = FPS from the average of the slowest 1% of frames.
    Average,
}

// ---------------------------------------------------------------- sections

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub hotkey: String,
    pub show_on_start: bool,
    /// How often sensors and FPS text are refreshed.
    pub update_interval_ms: u32,
    /// Redraw rate of the frame-time graph while visible.
    pub graph_refresh_hz: u32,
    pub monitor: MonitorMode,
    pub anchor: Anchor,
    pub offset_x: i32,
    pub offset_y: i32,
    /// Extra scale on top of the monitor DPI scale ("overlay size").
    pub scale: f32,
    /// Opacity of the whole overlay, 0.1..=1.
    pub opacity: f32,
}

impl Default for General {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Alt+O".into(),
            show_on_start: true,
            update_interval_ms: 500,
            graph_refresh_hz: 30,
            monitor: MonitorMode::Foreground,
            anchor: Anchor::TopLeft,
            offset_x: 16,
            offset_y: 16,
            scale: 1.0,
            opacity: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub layout: Layout,
    /// Font family name ("Segoe UI Semibold") or a path to a .ttf/.otf file.
    pub font: String,
    pub value_size: f32,
    pub label_size: f32,
    pub label_color: Color,
    pub value_color: Color,
    pub unit_color: Color,
    pub background: Color,
    pub corner_radius: f32,
    pub padding: f32,
    pub row_spacing: f32,
    pub column_spacing: f32,
    pub text_shadow: bool,
}

/// A good default overlay font that exists on a stock install of each OS.
pub fn default_font() -> &'static str {
    "Segoe UI Semibold"
}

impl Default for Style {
    fn default() -> Self {
        Self {
            layout: Layout::Vertical,
            font: default_font().into(),
            value_size: 16.0,
            label_size: 12.0,
            label_color: Color::rgba(0xFF, 0x5A, 0x5F, 0xFF),
            value_color: Color::rgba(0xFF, 0xFF, 0xFF, 0xFF),
            unit_color: Color::rgba(0xA0, 0xA6, 0xB0, 0xFF),
            background: Color::rgba(0x0B, 0x0D, 0x12, 0xB4),
            corner_radius: 8.0,
            padding: 10.0,
            row_spacing: 3.0,
            column_spacing: 14.0,
            text_shadow: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SensorItem {
    pub id: SensorId,
    pub enabled: bool,
    /// Empty = use the default label.
    #[serde(default)]
    pub label: String,
    /// Value colour override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
}

impl SensorItem {
    pub fn label(&self) -> &str {
        if self.label.is_empty() {
            self.id.default_label()
        } else {
            &self.label
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Graph {
    pub enabled: bool,
    pub width: f32,
    pub height: f32,
    pub mode: GraphMode,
    pub time_window_s: f32,
    pub frame_count: u32,
    pub scale: GraphScale,
    pub min_ms: f32,
    pub max_ms: f32,
    pub line_width: f32,
    pub line_color: Color,
    /// Frames slower than this are drawn with `warn_color`.
    pub warn_ms: f32,
    pub warn_color: Color,
    /// Frames slower than this are drawn with `bad_color`.
    pub bad_ms: f32,
    pub bad_color: Color,
    pub fill: bool,
    pub fill_opacity: f32,
    pub background: Color,
    /// Horizontal reference lines, in ms (16.67 = 60 FPS).
    pub grid_ms: Vec<f32>,
    pub grid_color: Color,
    pub show_stats: bool,
    pub show_scale: bool,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            enabled: true,
            width: 240.0,
            height: 64.0,
            mode: GraphMode::Time,
            time_window_s: 5.0,
            frame_count: 300,
            scale: GraphScale::Auto,
            min_ms: 0.0,
            max_ms: 33.4,
            line_width: 1.5,
            line_color: Color::rgba(0x3D, 0xDC, 0x84, 0xFF),
            warn_ms: 33.4,
            warn_color: Color::rgba(0xFF, 0xC1, 0x07, 0xFF),
            bad_ms: 50.0,
            bad_color: Color::rgba(0xFF, 0x3B, 0x3B, 0xFF),
            fill: true,
            fill_opacity: 0.22,
            background: Color::rgba(0x00, 0x00, 0x00, 0x50),
            grid_ms: vec![16.67, 33.33],
            grid_color: Color::rgba(0xFF, 0xFF, 0xFF, 0x30),
            show_stats: true,
            show_scale: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// Window used for FPS and average frame time.
    pub fps_window_ms: u32,
    /// Window used for 1% / 0.1% lows.
    pub lows_window_s: f32,
    pub lows_method: LowsMethod,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            fps_window_ms: 1000,
            lows_window_s: 10.0,
            lows_method: LowsMethod::Percentile,
        }
    }
}

/// A history graph of any sensor (GPU temperature, CPU load, FPS, …),
/// drawn under the frame-time graph. Sampled once per sensor tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SensorGraph {
    pub sensor: SensorId,
    pub enabled: bool,
    /// Graph height in px (width follows the frame-time graph).
    pub height: f32,
    /// Seconds of history shown.
    pub window_s: f32,
    pub color: Color,
    pub line_width: f32,
    pub fill: bool,
    /// Auto: fit the visible values; otherwise `min`..`max`.
    pub auto_scale: bool,
    pub min: f32,
    pub max: f32,
    /// Label and current value above the graph.
    pub show_title: bool,
}

impl Default for SensorGraph {
    fn default() -> Self {
        Self {
            sensor: SensorId::GpuTemp,
            enabled: true,
            height: 36.0,
            window_s: 60.0,
            color: Color::rgba(0x4F, 0x9D, 0xFF, 0xFF),
            line_width: 1.5,
            fill: true,
            auto_scale: true,
            min: 0.0,
            max: 100.0,
            show_title: true,
        }
    }
}

impl SensorGraph {
    pub fn for_sensor(sensor: SensorId) -> Self {
        let (min, max, color) = match sensor.unit() {
            "%" => (0.0, 100.0, Color::rgba(0x3D, 0xDC, 0x84, 0xFF)),
            "°C" => (20.0, 100.0, Color::rgba(0xFF, 0x8A, 0x3D, 0xFF)),
            "W" => (0.0, 300.0, Color::rgba(0xFF, 0xC1, 0x07, 0xFF)),
            _ => (0.0, 100.0, Color::rgba(0x4F, 0x9D, 0xFF, 0xFF)),
        };
        Self {
            sensor,
            min,
            max,
            color,
            auto_scale: sensor.unit() != "%",
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------- settings app

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiTheme {
    Dark,
    Light,
    /// Follow the OS setting.
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiAccent {
    Black,
    White,
    DarkRed,
    Gray,
    DarkBlue,
    Emerald,
    Violet,
    Amber,
}

impl UiAccent {
    pub const ALL: &'static [UiAccent] = &[
        UiAccent::DarkRed,
        UiAccent::DarkBlue,
        UiAccent::Gray,
        UiAccent::Black,
        UiAccent::White,
        UiAccent::Emerald,
        UiAccent::Violet,
        UiAccent::Amber,
    ];

    pub fn name(self) -> &'static str {
        match self {
            UiAccent::Black => "Чёрный",
            UiAccent::White => "Белый",
            UiAccent::DarkRed => "Тёмно-красный",
            UiAccent::Gray => "Серый",
            UiAccent::DarkBlue => "Тёмно-синий",
            UiAccent::Emerald => "Изумрудный",
            UiAccent::Violet => "Фиолетовый",
            UiAccent::Amber => "Янтарный",
        }
    }

    /// Base accent colour (r, g, b).
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            UiAccent::Black => (0x0A, 0x0A, 0x0A),
            UiAccent::White => (0xF4, 0xF4, 0xF4),
            UiAccent::DarkRed => (0x9B, 0x1B, 0x23),
            UiAccent::Gray => (0x6B, 0x6E, 0x74),
            UiAccent::DarkBlue => (0x1D, 0x3A, 0x8A),
            UiAccent::Emerald => (0x0F, 0x7A, 0x55),
            UiAccent::Violet => (0x5B, 0x3C, 0xC4),
            UiAccent::Amber => (0xC2, 0x7A, 0x0E),
        }
    }
}

/// Look of the settings app itself (the overlay ignores this section).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub theme: UiTheme,
    pub accent: UiAccent,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            theme: UiTheme::Dark,
            accent: UiAccent::DarkRed,
        }
    }
}

// ---------------------------------------------------------------- root

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub style: Style,
    pub stats: Stats,
    pub graph: Graph,
    pub sensors: Vec<SensorItem>,
    pub sensor_graphs: Vec<SensorGraph>,
    pub ui: Ui,
}

const DEFAULT_ON: &[SensorId] = &[
    SensorId::Fps,
    SensorId::FrameTime,
    SensorId::Low1,
    SensorId::GpuLoad,
    SensorId::GpuTemp,
    SensorId::GpuClock,
    SensorId::GpuPower,
    SensorId::GpuVram,
    SensorId::CpuLoad,
    SensorId::CpuClock,
    SensorId::CpuTemp,
    SensorId::RamUsed,
];

impl Default for Config {
    fn default() -> Self {
        let mut c = Self {
            general: General::default(),
            style: Style::default(),
            stats: Stats::default(),
            graph: Graph::default(),
            sensors: Vec::new(),
            sensor_graphs: Vec::new(),
            ui: Ui::default(),
        };
        c.normalize();
        c
    }
}

impl Config {
    /// Make the sensor list contain every known sensor exactly once
    /// (keeps user order; new sensors are appended).
    pub fn normalize(&mut self) {
        let mut seen = Vec::new();
        self.sensors.retain(|s| {
            if seen.contains(&s.id) {
                false
            } else {
                seen.push(s.id);
                true
            }
        });
        // Default-on sensors first (in DEFAULT_ON order), then the rest.
        let fresh = self.sensors.is_empty();
        let order = DEFAULT_ON
            .iter()
            .copied()
            .chain(SensorId::ALL.iter().copied());
        for id in order {
            if !seen.contains(&id) {
                seen.push(id);
                self.sensors.push(SensorItem {
                    id,
                    enabled: fresh && DEFAULT_ON.contains(&id),
                    label: String::new(),
                    color: None,
                });
            }
        }
        let g = &mut self.general;
        g.update_interval_ms = g.update_interval_ms.clamp(50, 10_000);
        g.graph_refresh_hz = g.graph_refresh_hz.clamp(1, 240);
        g.scale = g.scale.clamp(0.25, 8.0);
        g.opacity = g.opacity.clamp(0.1, 1.0);
        for sg in &mut self.sensor_graphs {
            sg.height = sg.height.clamp(8.0, 1000.0);
            sg.window_s = sg.window_s.clamp(2.0, 3600.0);
            sg.line_width = sg.line_width.clamp(0.5, 8.0);
            if sg.max <= sg.min {
                sg.max = sg.min + 1.0;
            }
        }
        let gr = &mut self.graph;
        gr.time_window_s = gr.time_window_s.clamp(0.5, 120.0);
        gr.frame_count = gr.frame_count.clamp(10, 20_000);
        gr.width = gr.width.clamp(20.0, 4000.0);
        gr.height = gr.height.clamp(10.0, 2000.0);
        if gr.max_ms <= gr.min_ms {
            gr.max_ms = gr.min_ms + 1.0;
        }
        self.stats.fps_window_ms = self.stats.fps_window_ms.clamp(100, 60_000);
        self.stats.lows_window_s = self.stats.lows_window_s.clamp(1.0, 600.0);
    }

    /// Seconds of frame history the overlay must keep.
    pub fn history_seconds(&self) -> f64 {
        let g = if self.graph.mode == GraphMode::Time {
            self.graph.time_window_s as f64
        } else {
            30.0
        };
        g.max(self.stats.lows_window_s as f64)
            .max(self.stats.fps_window_ms as f64 / 1000.0)
            + 1.0
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let mut c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        c.normalize();
        Ok(c)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Load the config; on a missing file the default is written.
    /// On a broken file the default is returned together with the error.
    pub fn load() -> (Self, Option<String>) {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match Self::from_toml(&text) {
                Ok(c) => (c, None),
                Err(e) => (Self::default(), Some(format!("{}: {e}", path.display()))),
            },
            Err(_) => {
                let c = Self::default();
                let _ = c.save();
                (c, None)
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        // Write to a temp file and rename so the overlay never reads half a file.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
}

pub fn config_dir() -> PathBuf {
    {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("UniversalOverlay")
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let c = Config::default();
        let back = Config::from_toml(&c.to_toml()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn partial_config_fills_defaults() {
        let c = Config::from_toml("[general]\nhotkey = \"Alt+F1\"\n").unwrap();
        assert_eq!(c.general.hotkey, "Alt+F1");
        assert_eq!(c.sensors.len(), SensorId::COUNT);
    }

    #[test]
    fn sensor_graphs_roundtrip() {
        let mut c = Config::default();
        c.sensor_graphs
            .push(SensorGraph::for_sensor(SensorId::CpuLoad));
        c.ui.theme = UiTheme::Light;
        let back = Config::from_toml(&c.to_toml()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn color_hex() {
        assert_eq!(
            Color::from_hex("#11223344"),
            Some(Color::rgba(0x11, 0x22, 0x33, 0x44))
        );
        assert_eq!(
            Color::from_hex("112233"),
            Some(Color::rgba(0x11, 0x22, 0x33, 0xFF))
        );
        assert_eq!(Color::from_hex("zz"), None);
    }
}
