//! Universal Overlay settings. Edits config.toml; every change is saved
//! (debounced) and the running overlay is told to reload, so the overlay
//! itself is the live preview.
#![windows_subsystem = "windows"]

mod theme;

use eframe::egui::{self, RichText};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use theme::{accent_hi, text, text_dim, text_faint};
use uo_common::config::*;
use uo_common::hotkey::{lookup_key, Hotkey};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Style,
    Sensors,
    Graph,
    Stats,
    Look,
}

const TABS: &[(Tab, &str)] = &[
    (Tab::General, "General"),
    (Tab::Style, "Style"),
    (Tab::Sensors, "Sensors"),
    (Tab::Graph, "Graphs"),
    (Tab::Stats, "Statistics"),
    (Tab::Look, "Appearance"),
];

struct App {
    cfg: Config,
    saved: Config,
    load_error: Option<String>,
    tab: Tab,
    fonts: Vec<String>,
    font_filter: String,
    capturing: bool,
    changed_at: Option<Instant>,
    /// Result of `universal-overlay-updater --check` (filled by a thread).
    update_check: Arc<Mutex<Option<String>>>,
    checking: bool,
    status: String,
    /// Theme currently applied to egui: (theme, accent, OS dark mode).
    applied_theme: Option<(UiTheme, UiAccent, bool)>,
    /// Sensor chosen in the "add graph" picker.
    new_graph: uo_common::sensor::SensorId,
    /// When the current tab was opened (content fade-in).
    tab_since: Instant,
    /// Background before the last theme switch and when it happened
    /// (cross-fade from the old theme).
    theme_fade: Option<(egui::Color32, Instant)>,
    /// When the status text was set (it fades out).
    status_since: Instant,
}

impl App {
    fn new() -> Self {
        let (cfg, err) = Config::load();
        let fonts = uo_common::fonts::list_fonts()
            .into_iter()
            .map(|f| f.name)
            .collect();
        Self {
            saved: cfg.clone(),
            cfg,
            load_error: err,
            tab: match std::env::args()
                .find_map(|a| a.strip_prefix("--tab=").map(str::to_owned))
                .as_deref()
            {
                Some("style") => Tab::Style,
                Some("sensors") => Tab::Sensors,
                Some("graph") => Tab::Graph,
                Some("stats") => Tab::Stats,
                Some("look") => Tab::Look,
                _ => Tab::General,
            },
            fonts,
            font_filter: String::new(),
            capturing: false,
            changed_at: None,
            status: String::new(),
            update_check: Arc::new(Mutex::new(None)),
            checking: false,
            applied_theme: None,
            new_graph: uo_common::sensor::SensorId::GpuTemp,
            tab_since: Instant::now(),
            theme_fade: None,
            status_since: Instant::now(),
        }
    }

    fn apply(&mut self) {
        self.cfg.normalize();
        match self.cfg.save() {
            Ok(()) => {
                self.saved = self.cfg.clone();
                self.status_since = Instant::now();
                self.status = if overlay::notify_reload() {
                    "Saved".into()
                } else {
                    "Saved · overlay is not running".into()
                };
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }
}

// ---------------------------------------------------------------- helpers

fn color_edit(ui: &mut egui::Ui, c: &mut Color) {
    let mut a = [c.r, c.g, c.b, c.a];
    ui.color_edit_button_srgba_unmultiplied(&mut a);
    *c = Color::rgba(a[0], a[1], a[2], a[3]);
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(label).color(text_dim()));
    ui.horizontal(add);
    ui.end_row();
}

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .min_col_width(210.0)
        .spacing([16.0, 12.0])
        .show(ui, add);
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(12.0).color(text_faint()));
}

impl App {
    fn hotkey_ui(&mut self, ui: &mut egui::Ui) {
        if self.capturing {
            let events = ui.input(|i| i.events.clone());
            for ev in events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = ev
                {
                    if key == egui::Key::Escape {
                        self.capturing = false;
                        break;
                    }
                    if let Some((name, _)) = lookup_key(key.name()) {
                        let hk = Hotkey {
                            ctrl: modifiers.ctrl,
                            alt: modifiers.alt,
                            shift: modifiers.shift,
                            win: modifiers.mac_cmd,
                            key: name,
                        };
                        self.cfg.general.hotkey = hk.to_string();
                        self.capturing = false;
                        break;
                    }
                }
            }
            theme::primary(ui, "press a hotkey · Esc to cancel");
        } else {
            if ui
                .add(egui::Button::new(
                    RichText::new(&self.cfg.general.hotkey)
                        .monospace()
                        .color(text()),
                ))
                .on_hover_text("Click to record a new hotkey")
                .clicked()
            {
                self.capturing = true;
            }
            if Hotkey::parse(&self.cfg.general.hotkey).is_none() {
                ui.label(RichText::new("not recognized").color(accent_hi()));
            }
        }
    }

    fn tab_general(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Controls");
        grid(ui, "g1", |ui| {
            row(ui, "Hotkey", |ui| self.hotkey_ui(ui));
            row(ui, "Show on start", |ui| {
                theme::toggle(ui, &mut self.cfg.general.show_on_start);
            });
        });
        let g = &mut self.cfg.general;
        theme::section(ui, "Position");
        grid(ui, "g2", |ui| {
            row(ui, "Monitor", |ui| {
                ui.selectable_value(&mut g.monitor, MonitorMode::Foreground, "With the game");
                ui.selectable_value(&mut g.monitor, MonitorMode::Primary, "Primary");
            });
            row(ui, "Corner", |ui| {
                egui::ComboBox::from_id_salt("anchor")
                    .selected_text(g.anchor.name())
                    .show_ui(ui, |ui| {
                        for a in Anchor::ALL {
                            ui.selectable_value(&mut g.anchor, *a, a.name());
                        }
                    });
            });
            row(ui, "Offset", |ui| {
                ui.add(
                    egui::DragValue::new(&mut g.offset_x)
                        .range(-4000..=4000)
                        .prefix("x "),
                );
                ui.add(
                    egui::DragValue::new(&mut g.offset_y)
                        .range(-4000..=4000)
                        .prefix("y "),
                );
            });
            row(ui, "Overlay size", |ui| {
                ui.add(
                    egui::Slider::new(&mut g.scale, 0.25..=4.0)
                        .step_by(0.05)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
            row(ui, "Opacity", |ui| {
                ui.add(
                    egui::Slider::new(&mut g.opacity, 0.1..=1.0)
                        .step_by(0.05)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
        });
        theme::section(ui, "Refresh");
        grid(ui, "g3", |ui| {
            row(ui, "Sensors, ms", |ui| {
                ui.add(egui::Slider::new(&mut g.update_interval_ms, 100..=5000).logarithmic(true));
            });
            row(ui, "Graph, Hz", |ui| {
                ui.add(egui::Slider::new(&mut g.graph_refresh_hz, 1..=144));
            });
        });
        note(ui, "The size is multiplied by the display scale set in the system. The overlay never takes clicks or focus.");
        self.updates_ui(ui);
    }

    fn tab_style(&mut self, ui: &mut egui::Ui) {
        let fonts = &self.fonts;
        let filter = &mut self.font_filter;
        let s = &mut self.cfg.style;
        theme::section(ui, "Text");
        grid(ui, "s1", |ui| {
            row(ui, "Layout", |ui| {
                ui.selectable_value(&mut s.layout, Layout::Vertical, "Column");
                ui.selectable_value(&mut s.layout, Layout::Horizontal, "Row");
            });
            row(ui, "Font", |ui| {
                egui::ComboBox::from_id_salt("font")
                    .width(240.0)
                    .height(380.0)
                    .selected_text(s.font.clone())
                    .show_ui(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(filter).hint_text("search"));
                        let f = filter.to_lowercase();
                        for name in fonts
                            .iter()
                            .filter(|n| f.is_empty() || n.to_lowercase().contains(&f))
                        {
                            ui.selectable_value(&mut s.font, name.clone(), name);
                        }
                    });
            });
            row(ui, "Value size", |ui| {
                ui.add(egui::Slider::new(&mut s.value_size, 8.0..=48.0).step_by(0.5));
            });
            row(ui, "Label size", |ui| {
                ui.add(egui::Slider::new(&mut s.label_size, 6.0..=36.0).step_by(0.5));
            });
            row(ui, "Shadow", |ui| {
                theme::toggle(ui, &mut s.text_shadow);
            });
        });
        theme::section(ui, "Colors");
        grid(ui, "s2", |ui| {
            row(ui, "Labels", |ui| color_edit(ui, &mut s.label_color));
            row(ui, "Values", |ui| color_edit(ui, &mut s.value_color));
            row(ui, "Units", |ui| color_edit(ui, &mut s.unit_color));
            row(ui, "Background", |ui| color_edit(ui, &mut s.background));
        });
        theme::section(ui, "Geometry");
        grid(ui, "s3", |ui| {
            row(ui, "Corner radius", |ui| {
                ui.add(egui::Slider::new(&mut s.corner_radius, 0.0..=30.0));
            });
            row(ui, "Padding", |ui| {
                ui.add(egui::Slider::new(&mut s.padding, 0.0..=40.0));
            });
            row(ui, "Row spacing", |ui| {
                ui.add(egui::Slider::new(&mut s.row_spacing, 0.0..=30.0));
            });
            row(ui, "Column spacing", |ui| {
                ui.add(egui::Slider::new(&mut s.column_spacing, 0.0..=60.0));
            });
        });
    }

    fn tab_sensors(&mut self, ui: &mut egui::Ui) {
        note(
            ui,
            "The list order is the overlay order. An empty label means the default label.",
        );
        ui.add_space(6.0);
        let n = self.cfg.sensors.len();
        let mut swap: Option<(usize, usize)> = None;
        egui::Grid::new("sensors")
            .num_columns(5)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                for h in ["", "Sensor", "Label", "Color", ""] {
                    ui.label(
                        RichText::new(h.to_uppercase())
                            .size(11.0)
                            .color(text_faint())
                            .extra_letter_spacing(1.2),
                    );
                }
                ui.end_row();
                for (i, s) in self.cfg.sensors.iter_mut().enumerate() {
                    theme::toggle(ui, &mut s.enabled);
                    ui.label(RichText::new(s.id.description()).color(if s.enabled {
                        text()
                    } else {
                        text_faint()
                    }));
                    ui.add_sized(
                        [120.0, 22.0],
                        egui::TextEdit::singleline(&mut s.label).hint_text(s.id.default_label()),
                    );
                    ui.horizontal(|ui| {
                        let mut custom = s.color.is_some();
                        if theme::toggle(ui, &mut custom).changed() {
                            s.color = if custom {
                                Some(Color::rgba(0xFF, 0xFF, 0xFF, 0xFF))
                            } else {
                                None
                            };
                        }
                        if let Some(c) = s.color.as_mut() {
                            color_edit(ui, c);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if theme::arrow(ui, true, i > 0) {
                            swap = Some((i, i - 1));
                        }
                        if theme::arrow(ui, false, i + 1 < n) {
                            swap = Some((i, i + 1));
                        }
                    });
                    ui.end_row();
                }
            });
        if let Some((a, b)) = swap {
            self.cfg.sensors.swap(a, b);
        }
    }

    fn tab_graph(&mut self, ui: &mut egui::Ui) {
        let g = &mut self.cfg.graph;
        theme::section(ui, "Frame-time graph");
        grid(ui, "gr1", |ui| {
            row(ui, "Show", |ui| {
                theme::toggle(ui, &mut g.enabled);
            });
            row(ui, "Size", |ui| {
                ui.add(
                    egui::DragValue::new(&mut g.width)
                        .range(40.0..=2000.0)
                        .suffix(" px"),
                );
                ui.label(RichText::new("×").color(text_faint()));
                ui.add(
                    egui::DragValue::new(&mut g.height)
                        .range(16.0..=1000.0)
                        .suffix(" px"),
                );
            });
            row(ui, "X axis", |ui| {
                ui.selectable_value(&mut g.mode, GraphMode::Time, "Time");
                ui.selectable_value(&mut g.mode, GraphMode::Frames, "Frames");
            });
            match g.mode {
                GraphMode::Time => row(ui, "Window", |ui| {
                    ui.add(
                        egui::Slider::new(&mut g.time_window_s, 1.0..=60.0)
                            .step_by(0.5)
                            .suffix(" s"),
                    );
                }),
                GraphMode::Frames => row(ui, "Window", |ui| {
                    ui.add(
                        egui::Slider::new(&mut g.frame_count, 30..=5000)
                            .logarithmic(true)
                            .suffix(" frames"),
                    );
                }),
            }
            row(ui, "Y scale", |ui| {
                ui.selectable_value(&mut g.scale, GraphScale::Auto, "Auto");
                ui.selectable_value(&mut g.scale, GraphScale::Fixed, "Fixed");
            });
            if g.scale == GraphScale::Fixed {
                row(ui, "Range", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut g.min_ms)
                            .range(0.0..=1000.0)
                            .speed(0.1)
                            .suffix(" ms"),
                    );
                    ui.label(RichText::new("—").color(text_faint()));
                    ui.add(
                        egui::DragValue::new(&mut g.max_ms)
                            .range(1.0..=2000.0)
                            .speed(0.1)
                            .suffix(" ms"),
                    );
                });
            }
        });
        theme::section(ui, "Line");
        grid(ui, "gr2", |ui| {
            row(ui, "Width", |ui| {
                ui.add(egui::Slider::new(&mut g.line_width, 0.5..=6.0).step_by(0.5));
            });
            row(ui, "Color", |ui| color_edit(ui, &mut g.line_color));
            row(ui, "Warning from", |ui| {
                ui.add(
                    egui::DragValue::new(&mut g.warn_ms)
                        .range(1.0..=1000.0)
                        .speed(0.1)
                        .suffix(" ms"),
                );
                color_edit(ui, &mut g.warn_color);
            });
            row(ui, "Bad frame from", |ui| {
                ui.add(
                    egui::DragValue::new(&mut g.bad_ms)
                        .range(1.0..=2000.0)
                        .speed(0.1)
                        .suffix(" ms"),
                );
                color_edit(ui, &mut g.bad_color);
            });
            row(ui, "Fill", |ui| {
                theme::toggle(ui, &mut g.fill);
                if g.fill {
                    ui.add(egui::Slider::new(&mut g.fill_opacity, 0.0..=1.0));
                }
            });
            row(ui, "Background", |ui| color_edit(ui, &mut g.background));
        });
        theme::section(ui, "Grid and labels");
        grid(ui, "gr3", |ui| {
            let mut remove = None;
            for (i, v) in g.grid_ms.iter_mut().enumerate() {
                row(ui, "Line", |ui| {
                    ui.add(
                        egui::DragValue::new(v)
                            .range(0.1..=2000.0)
                            .speed(0.1)
                            .suffix(" ms"),
                    );
                    ui.label(RichText::new(format!("{:.0} FPS", 1000.0 / *v)).color(text_faint()));
                    if theme::link(ui, "remove") {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                g.grid_ms.remove(i);
            }
            row(ui, "Add line, FPS", |ui| {
                for (label, ms) in [
                    ("30", 33.33),
                    ("60", 16.67),
                    ("120", 8.33),
                    ("144", 6.94),
                    ("240", 4.17),
                ] {
                    if ui
                        .small_button(label)
                        .on_hover_text(format!("{ms} ms"))
                        .clicked()
                    {
                        g.grid_ms.push(ms);
                    }
                }
            });
            row(ui, "Grid color", |ui| color_edit(ui, &mut g.grid_color));
            row(ui, "Scale labels", |ui| {
                theme::toggle(ui, &mut g.show_scale);
            });
            row(ui, "avg / max above the graph", |ui| {
                theme::toggle(ui, &mut g.show_stats);
            });
        });
        self.sensor_graphs_ui(ui);
    }

    fn sensor_graphs_ui(&mut self, ui: &mut egui::Ui) {
        use uo_common::sensor::SensorId;
        theme::section(ui, "Sensor graphs");
        note(ui, "History of any sensor under the frame-time graph. Same width as the frame-time graph, one point per sensor interval.");
        ui.add_space(4.0);
        let graphable = |id: &SensorId| *id != SensorId::Clock;
        let n = self.cfg.sensor_graphs.len();
        let mut remove = None;
        let mut swap = None;
        for (i, sg) in self.cfg.sensor_graphs.iter_mut().enumerate() {
            egui::Frame::new()
                .fill(theme::p().surface)
                .corner_radius(4)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        theme::toggle(ui, &mut sg.enabled);
                        egui::ComboBox::from_id_salt(("sg", i))
                            .width(230.0)
                            .selected_text(sg.sensor.description())
                            .show_ui(ui, |ui| {
                                for id in SensorId::ALL.iter().filter(|id| graphable(id)) {
                                    ui.selectable_value(&mut sg.sensor, *id, id.description());
                                }
                            });
                        color_edit(ui, &mut sg.color);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::link(ui, "remove") {
                                remove = Some(i);
                            }
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if theme::arrow(ui, false, i + 1 < n) {
                                swap = Some((i, i + 1));
                            }
                            if theme::arrow(ui, true, i > 0) {
                                swap = Some((i, i - 1));
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("height").color(text_dim()));
                        ui.add(
                            egui::DragValue::new(&mut sg.height)
                                .range(8.0..=600.0)
                                .suffix(" px"),
                        );
                        ui.label(RichText::new("window").color(text_dim()));
                        ui.add(
                            egui::DragValue::new(&mut sg.window_s)
                                .range(2.0..=3600.0)
                                .suffix(" s"),
                        );
                        ui.label(RichText::new("line").color(text_dim()));
                        ui.add(
                            egui::DragValue::new(&mut sg.line_width)
                                .range(0.5..=8.0)
                                .speed(0.1),
                        );
                        ui.label(RichText::new("fill").color(text_dim()));
                        theme::toggle(ui, &mut sg.fill);
                        ui.label(RichText::new("label").color(text_dim()));
                        theme::toggle(ui, &mut sg.show_title);
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("scale").color(text_dim()));
                        ui.selectable_value(&mut sg.auto_scale, true, "Auto");
                        ui.selectable_value(&mut sg.auto_scale, false, "Fixed");
                        if !sg.auto_scale {
                            let unit = sg.sensor.unit();
                            ui.add(
                                egui::DragValue::new(&mut sg.min)
                                    .speed(0.5)
                                    .suffix(format!(" {unit}")),
                            );
                            ui.label(RichText::new("—").color(text_faint()));
                            ui.add(
                                egui::DragValue::new(&mut sg.max)
                                    .speed(0.5)
                                    .suffix(format!(" {unit}")),
                            );
                        }
                    });
                });
            ui.add_space(4.0);
        }
        if let Some(i) = remove {
            self.cfg.sensor_graphs.remove(i);
        }
        if let Some((a, b)) = swap {
            self.cfg.sensor_graphs.swap(a, b);
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("sg_new")
                .width(230.0)
                .selected_text(self.new_graph.description())
                .show_ui(ui, |ui| {
                    for id in SensorId::ALL.iter().filter(|id| graphable(id)) {
                        ui.selectable_value(&mut self.new_graph, *id, id.description());
                    }
                });
            if theme::primary(ui, "Add graph") {
                self.cfg
                    .sensor_graphs
                    .push(SensorGraph::for_sensor(self.new_graph));
            }
        });
    }

    fn tab_look(&mut self, ui: &mut egui::Ui) {
        let u = &mut self.cfg.ui;
        theme::section(ui, "Settings theme");
        grid(ui, "lk1", |ui| {
            row(ui, "Theme", |ui| {
                ui.selectable_value(&mut u.theme, UiTheme::Dark, "Dark");
                ui.selectable_value(&mut u.theme, UiTheme::Light, "Light");
                ui.selectable_value(&mut u.theme, UiTheme::System, "System");
            });
            row(ui, "Accent", |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for a in UiAccent::ALL {
                    let (r, g, b) = a.rgb();
                    if theme::swatch(
                        ui,
                        egui::Color32::from_rgb(r, g, b),
                        u.accent == *a,
                        a.name(),
                    ) {
                        u.accent = *a;
                    }
                }
            });
            row(ui, "", |ui| {
                ui.label(RichText::new(u.accent.name()).color(text_dim()));
            });
        });
        note(ui, "Theme and accent change only this settings window. The overlay's own colors are on the Style and Graphs tabs.");
    }

    fn tab_stats(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.cfg.stats;
        theme::section(ui, "Calculation");
        grid(ui, "st", |ui| {
            row(ui, "FPS window", |ui| {
                ui.add(
                    egui::Slider::new(&mut s.fps_window_ms, 100..=5000)
                        .logarithmic(true)
                        .suffix(" ms"),
                );
            });
            row(ui, "1% / 0.1% low window", |ui| {
                ui.add(
                    egui::Slider::new(&mut s.lows_window_s, 1.0..=120.0)
                        .logarithmic(true)
                        .suffix(" s"),
                );
            });
            row(ui, "Lows method", |ui| {
                ui.selectable_value(&mut s.lows_method, LowsMethod::Percentile, "Percentile");
                ui.selectable_value(&mut s.lows_method, LowsMethod::Average, "Average of worst");
            });
        });
        note(
            ui,
            "FPS is the number of frames divided by their total time in the window. Frame time is the interval between \
             Present calls of one swap chain, timed with QPC from ETW, as in PresentMon. \
             Percentile: 1% low = FPS at the 99th percentile of frame time.",
        );
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::bg().to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let os_dark = ui
            .ctx()
            .system_theme()
            .is_none_or(|t| t == egui::Theme::Dark);
        let want = (self.cfg.ui.theme, self.cfg.ui.accent, os_dark);
        if self.applied_theme != Some(want) {
            if self.applied_theme.is_some() {
                self.theme_fade = Some((theme::bg(), Instant::now()));
            }
            theme::apply(ui.ctx(), want.0, want.1);
            self.applied_theme = Some(want);
        }
        const FADE: f32 = 0.28;
        // Debounced auto-apply.
        if self.cfg != self.saved {
            let t = *self.changed_at.get_or_insert_with(Instant::now);
            if t.elapsed() >= Duration::from_millis(250) {
                self.apply();
                self.changed_at = None;
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(260));
            }
        } else {
            self.changed_at = None;
        }

        let bar = egui::Frame::new()
            .fill(theme::bg())
            .inner_margin(egui::Margin::symmetric(24, 16));
        egui::Panel::top("tabs")
            .frame(bar)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("UNIVERSAL OVERLAY")
                            .size(12.0)
                            .color(text())
                            .extra_letter_spacing(2.0)
                            .strong(),
                    );
                    ui.add_space(18.0);
                    ui.spacing_mut().item_spacing.x = 18.0;
                    let mut selected_rect = None;
                    for (t, name) in TABS {
                        let resp = theme::tab(ui, self.tab == *t, name);
                        if resp.clicked() && self.tab != *t {
                            self.tab = *t;
                            self.tab_since = Instant::now();
                        }
                        if self.tab == *t {
                            selected_rect = Some(resp.rect);
                        }
                    }
                    if let Some(r) = selected_rect {
                        theme::tab_underline(ui, r);
                    }
                });
            });
        let foot = egui::Frame::new()
            .fill(theme::bg())
            .inner_margin(egui::Margin::symmetric(24, 12));
        egui::Panel::bottom("status")
            .frame(foot)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match &self.load_error {
                        Some(e) => ui.label(
                            RichText::new(format!("config.toml is damaged: {e}"))
                                .size(12.0)
                                .color(accent_hi()),
                        ),
                        None => {
                            // Visible for 2 s, then fades out over 0.6 s.
                            let age = self.status_since.elapsed().as_secs_f32();
                            let a = 1.0 - ((age - 2.0) / 0.6).clamp(0.0, 1.0);
                            if a > 0.0 && a < 1.0 {
                                ui.ctx().request_repaint();
                            } else if a >= 1.0 {
                                ui.ctx().request_repaint_after(Duration::from_secs_f32(
                                    (2.0 - age).max(0.05),
                                ));
                            }
                            ui.label(
                                RichText::new(&self.status)
                                    .size(12.0)
                                    .color(text_faint().gamma_multiply(a)),
                            )
                        }
                    };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 18.0;
                        if theme::link(ui, "Reset all") {
                            self.cfg = Config::default();
                        }
                        if theme::link(ui, "Config folder") {
                            let _ = std::fs::create_dir_all(config_dir());
                            overlay::open_path(&config_dir());
                        }
                        if !overlay::is_running() && theme::link(ui, "Start overlay") {
                            overlay::launch();
                        }
                    });
                });
            });
        let body = egui::Frame::new()
            .fill(theme::bg())
            .inner_margin(egui::Margin {
                left: 24,
                right: 24,
                top: 0,
                bottom: 8,
            });
        let t = theme::ease_out(self.tab_since.elapsed().as_secs_f32() / 0.22);
        if t < 1.0 {
            ui.ctx().request_repaint();
        }
        egui::CentralPanel::default().frame(body).show(ui, |ui| {
            ui.multiply_opacity(0.15 + 0.85 * t);
            ui.add_space((1.0 - t) * 10.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match self.tab {
                    Tab::General => self.tab_general(ui),
                    Tab::Style => self.tab_style(ui),
                    Tab::Sensors => self.tab_sensors(ui),
                    Tab::Graph => self.tab_graph(ui),
                    Tab::Stats => self.tab_stats(ui),
                    Tab::Look => self.tab_look(ui),
                });
        });
        if let Some((old, since)) = self.theme_fade {
            let k = since.elapsed().as_secs_f32() / FADE;
            if k >= 1.0 {
                self.theme_fade = None;
            } else {
                let a = 1.0 - theme::ease_out(k);
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::unique("theme-fade"),
                ));
                painter.rect_filled(ui.ctx().content_rect(), 0.0, old.gamma_multiply(a));
                ui.ctx().request_repaint();
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.cfg != self.saved {
            self.apply();
        }
    }
}

// ---------------------------------------------------------------- overlay IPC

mod overlay {
    use uo_common::{MSG_RELOAD, OVERLAY_EXE, OVERLAY_WINDOW_CLASS};
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

    fn window() -> windows_sys::Win32::Foundation::HWND {
        let cls: Vec<u16> = OVERLAY_WINDOW_CLASS.encode_utf16().chain(Some(0)).collect();
        unsafe { FindWindowW(cls.as_ptr(), std::ptr::null()) }
    }

    pub fn is_running() -> bool {
        !window().is_null()
    }

    pub fn notify_reload() -> bool {
        let w = window();
        !w.is_null() && unsafe { PostMessageW(w, MSG_RELOAD, 0, 0) } != 0
    }

    pub fn launch() {
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        {
            let _ = std::process::Command::new(dir.join(format!("{OVERLAY_EXE}.exe"))).spawn();
        }
    }

    pub fn open_path(p: &std::path::Path) {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Universal Overlay Settings")
            .with_inner_size([720.0, 640.0])
            .with_min_inner_size([560.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Universal Overlay Settings",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}

// ---------------------------------------------------------------- updates

fn updater_path() -> std::path::PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    dir.join(format!(
        "{}{}",
        uo_common::UPDATER_EXE,
        std::env::consts::EXE_SUFFIX
    ))
}

impl App {
    fn updates_ui(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Updates");
        let result = self.update_check.lock().map(|g| g.clone()).unwrap_or(None);
        if result.is_some() {
            self.checking = false;
        }
        grid(ui, "upd", |ui| {
            row(ui, "Version", |ui| {
                ui.label(RichText::new(uo_common::VERSION).color(text()));
            });
            row(ui, "Latest version", |ui| {
                if self.checking {
                    ui.spinner();
                    ui.label(RichText::new("checking…").color(text_dim()));
                } else {
                    match result.as_deref() {
                        Some(r) if r.starts_with("available ") => {
                            let v = r.trim_start_matches("available ").to_string();
                            ui.label(RichText::new(format!("{v} available")).color(text()));
                            if theme::primary(ui, "Update") {
                                let _ = std::process::Command::new(updater_path()).spawn();
                                std::process::exit(0); // the installer restarts everything
                            }
                        }
                        Some(r) if r.starts_with("latest ") => {
                            ui.label(RichText::new("up to date").color(text_dim()));
                        }
                        Some(r) => {
                            ui.label(
                                RichText::new(r.trim_start_matches("error ")).color(accent_hi()),
                            );
                        }
                        None => {}
                    }
                    if theme::link(ui, "check") {
                        self.checking = true;
                        let slot = self.update_check.clone();
                        if let Ok(mut g) = slot.lock() {
                            *g = None;
                        }
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let mut cmd = std::process::Command::new(updater_path());
                            cmd.arg("--check");
                            {
                                use std::os::windows::process::CommandExt;
                                cmd.creation_flags(0x0800_0000); // no console window
                            }
                            let text = match cmd.output() {
                                Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
                                Err(e) => format!("error {e}"),
                            };
                            if let Ok(mut g) = slot.lock() {
                                *g = Some(text);
                            }
                            ctx.request_repaint();
                        });
                    }
                }
            });
        });
        note(ui, "Updates download the latest GitHub release, verify its SHA-256 and replace only the program — your settings are kept.");
    }
}
