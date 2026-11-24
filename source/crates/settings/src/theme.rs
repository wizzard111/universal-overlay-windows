//! Two themes (dark / light, or follow the OS) with a choice of accent
//! colours, and a few small custom widgets. The palette is a global that
//! [`apply`] swaps at runtime, so switching is instant.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Response, RichText, Sense, Shadow, Stroke, TextStyle, Ui,
};
use std::sync::RwLock;
use uo_common::config::{UiAccent, UiTheme};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub surface_hi: Color32,
    pub pressed: Color32,
    pub line: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub disabled: Color32,
    /// Fill of active controls (switches, selection, primary button).
    pub accent: Color32,
    /// Accent for thin lines and text (tab underline, links, errors);
    /// falls back to the text colour when the accent would be invisible.
    pub accent_hi: Color32,
    /// Text drawn on top of `accent`.
    pub on_accent: Color32,
}

const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x0C, 0x0C, 0x0C),
    surface: Color32::from_rgb(0x17, 0x17, 0x17),
    surface_hi: Color32::from_rgb(0x23, 0x23, 0x23),
    pressed: Color32::from_rgb(0x2C, 0x2C, 0x2C),
    line: Color32::from_rgb(0x26, 0x26, 0x26),
    text: Color32::from_rgb(0xEC, 0xEC, 0xEC),
    text_dim: Color32::from_rgb(0x9A, 0x9A, 0x9A),
    text_faint: Color32::from_rgb(0x60, 0x60, 0x60),
    disabled: Color32::from_rgb(0x2E, 0x2E, 0x2E),
    accent: Color32::from_rgb(0x9B, 0x1B, 0x23),
    accent_hi: Color32::from_rgb(0xB8, 0x24, 0x2D),
    on_accent: Color32::WHITE,
};

const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xF7, 0xF7, 0xF7),
    surface: Color32::from_rgb(0xEA, 0xEA, 0xEA),
    surface_hi: Color32::from_rgb(0xDE, 0xDE, 0xDE),
    pressed: Color32::from_rgb(0xD2, 0xD2, 0xD2),
    line: Color32::from_rgb(0xD8, 0xD8, 0xD8),
    text: Color32::from_rgb(0x12, 0x12, 0x12),
    text_dim: Color32::from_rgb(0x55, 0x55, 0x55),
    text_faint: Color32::from_rgb(0x8E, 0x8E, 0x8E),
    disabled: Color32::from_rgb(0xC8, 0xC8, 0xC8),
    accent: Color32::from_rgb(0x9B, 0x1B, 0x23),
    accent_hi: Color32::from_rgb(0x9B, 0x1B, 0x23),
    on_accent: Color32::WHITE,
};

static PALETTE: RwLock<Palette> = RwLock::new(DARK);

pub fn p() -> Palette {
    *PALETTE.read().unwrap_or_else(|e| e.into_inner())
}

fn luma(c: Color32) -> f32 {
    (0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32) / 255.0
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn build(dark: bool, accent: UiAccent) -> Palette {
    let mut pl = if dark { DARK } else { LIGHT };
    let (r, g, b) = accent.rgb();
    let a = Color32::from_rgb(r, g, b);
    pl.accent = a;
    pl.on_accent = if luma(a) > 0.55 {
        Color32::from_rgb(0x10, 0x10, 0x10)
    } else {
        Color32::WHITE
    };
    // Thin accent lines must stand out from the background.
    let contrast = (luma(a) - luma(pl.bg)).abs();
    pl.accent_hi = if contrast < 0.25 {
        pl.text
    } else if dark {
        mix(a, Color32::WHITE, 0.15)
    } else {
        a
    };
    pl
}

pub fn bg() -> Color32 {
    p().bg
}
pub fn text() -> Color32 {
    p().text
}
pub fn text_dim() -> Color32 {
    p().text_dim
}
pub fn text_faint() -> Color32 {
    p().text_faint
}
pub fn accent_hi() -> Color32 {
    p().accent_hi
}

/// Apply theme + accent. `System` follows the OS (re-checked every frame
/// by the caller, so it also follows a later OS switch).
pub fn apply(ctx: &egui::Context, theme: UiTheme, accent: UiAccent) {
    let dark = match theme {
        UiTheme::Dark => true,
        UiTheme::Light => false,
        UiTheme::System => ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark),
    };
    let pl = build(dark, accent);
    *PALETTE.write().unwrap_or_else(|e| e.into_inner()) = pl;
    ctx.all_styles_mut(|s| {
        let r = CornerRadius::same(3);
        let v = &mut s.visuals;
        *v = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.dark_mode = dark;
        v.override_text_color = None;
        v.panel_fill = pl.bg;
        v.window_fill = pl.surface;
        v.window_stroke = Stroke::new(1.0, pl.line);
        v.window_shadow = Shadow::NONE;
        v.popup_shadow = Shadow::NONE;
        v.window_corner_radius = r;
        v.menu_corner_radius = r;
        v.extreme_bg_color = mix(pl.bg, pl.surface, 0.5);
        v.text_edit_bg_color = Some(pl.surface);
        v.faint_bg_color = mix(pl.bg, pl.surface, 0.4);
        v.code_bg_color = pl.surface;
        v.hyperlink_color = pl.accent_hi;
        v.warn_fg_color = Color32::from_rgb(0xD8, 0xA2, 0x3A);
        v.error_fg_color = pl.accent_hi;
        v.selection.bg_fill = pl.accent;
        v.selection.stroke = Stroke::new(1.0, pl.on_accent);
        v.slider_trailing_fill = true;
        v.striped = false;
        v.indent_has_left_vline = false;
        v.button_frame = true;

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = pl.bg;
        w.noninteractive.weak_bg_fill = pl.bg;
        w.noninteractive.bg_stroke = Stroke::new(1.0, pl.line);
        w.noninteractive.fg_stroke = Stroke::new(1.0, pl.text_dim);
        w.noninteractive.corner_radius = r;
        for (st, bg) in [
            (&mut w.inactive, pl.surface),
            (&mut w.hovered, pl.surface_hi),
            (&mut w.active, pl.pressed),
            (&mut w.open, pl.surface_hi),
        ] {
            st.bg_fill = bg;
            st.weak_bg_fill = bg;
            st.bg_stroke = Stroke::NONE;
            st.fg_stroke = Stroke::new(1.0, pl.text);
            st.corner_radius = r;
            st.expansion = 0.0;
        }
        // Black / white accents sit on a same-coloured background: outline
        // the slider handle and active widgets so they stay visible.
        if (luma(pl.accent) - luma(pl.bg)).abs() < 0.25 {
            w.active.bg_stroke = Stroke::new(1.0, pl.text_dim);
        }

        let sp = &mut s.spacing;
        sp.item_spacing = egui::vec2(10.0, 10.0);
        sp.button_padding = egui::vec2(10.0, 4.0);
        sp.interact_size.y = 24.0;
        sp.slider_width = 200.0;
        sp.combo_width = 220.0;

        s.text_styles
            .insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles
            .insert(TextStyle::Button, FontId::proportional(14.0));
        s.text_styles
            .insert(TextStyle::Small, FontId::proportional(11.0));
        s.text_styles
            .insert(TextStyle::Heading, FontId::proportional(18.0));
        s.text_styles
            .insert(TextStyle::Monospace, FontId::monospace(13.0));
    });
}

/// iOS-like switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let pl = p();
    let size = egui::vec2(32.0, 18.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(resp.id, *on);
        let track = if *on {
            pl.accent
        } else if resp.hovered() {
            pl.pressed
        } else {
            pl.surface_hi
        };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(9), track);
        if *on && (luma(pl.accent) - luma(pl.bg)).abs() < 0.25 {
            painter.rect_stroke(
                rect,
                CornerRadius::same(9),
                Stroke::new(1.0, pl.text_dim),
                egui::StrokeKind::Inside,
            );
        }
        let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), t);
        let knob = if *on { pl.on_accent } else { pl.text_dim };
        painter.circle_filled(egui::pos2(x, rect.center().y), 6.0, knob);
    }
    resp
}

/// Small up/down arrow drawn with the painter (no font glyph needed).
pub fn arrow(ui: &mut Ui, up: bool, enabled: bool) -> bool {
    let pl = p();
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(20.0, 20.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let color = if !enabled {
        pl.disabled
    } else if resp.hovered() {
        pl.text
    } else {
        pl.text_faint
    };
    let c = rect.center();
    let (dx, dy) = (4.5, 2.8);
    let pts = if up {
        vec![
            egui::pos2(c.x - dx, c.y + dy),
            egui::pos2(c.x + dx, c.y + dy),
            egui::pos2(c.x, c.y - dy),
        ]
    } else {
        vec![
            egui::pos2(c.x - dx, c.y - dy),
            egui::pos2(c.x + dx, c.y - dy),
            egui::pos2(c.x, c.y + dy),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    enabled && resp.clicked()
}

/// Text-only tab; its colour fades between states. The accent underline
/// is drawn by [`tab_underline`], which slides between tabs.
pub fn tab(ui: &mut Ui, selected: bool, text: &str) -> Response {
    let pl = p();
    let resp = ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(14.0)
                .color(egui::Color32::TRANSPARENT),
        )
        .sense(Sense::click())
        .selectable(false),
    );
    let t_sel = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("sel"), selected, 0.18);
    let t_hov = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.12);
    let base = mix(pl.text_faint, pl.text_dim, t_hov);
    let color = mix(base, pl.text, t_sel);
    ui.painter().text(
        resp.rect.left_top(),
        egui::Align2::LEFT_TOP,
        text,
        FontId::proportional(14.0),
        color,
    );
    resp
}

/// Accent underline under the selected tab, sliding when the tab changes.
pub fn tab_underline(ui: &Ui, rect: egui::Rect) {
    let pl = p();
    let id = egui::Id::unique("tab-underline");
    let ctx = ui.ctx();
    let l = ctx.animate_value_with_time(id.with("l"), rect.left(), 0.22);
    let r = ctx.animate_value_with_time(id.with("r"), rect.right(), 0.22);
    let y = rect.bottom() + 6.0;
    ui.painter().line_segment(
        [egui::pos2(l, y), egui::pos2(r, y)],
        Stroke::new(2.0, pl.accent_hi),
    );
}

/// Ease-out cubic, 0..=1.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Small uppercase section title with a hairline under it.
pub fn section(ui: &mut Ui, title: &str) {
    let pl = p();
    ui.add_space(8.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(11.0)
            .color(pl.text_faint)
            .extra_letter_spacing(1.2),
    );
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.left()..=r.right(),
        r.top() - 4.0,
        Stroke::new(1.0, pl.line),
    );
    ui.add_space(2.0);
}

/// Flat text button (no frame).
pub fn link(ui: &mut Ui, text: &str) -> bool {
    let pl = p();
    let resp = ui.add(
        egui::Label::new(RichText::new(text).size(13.0).color(pl.text_dim))
            .sense(Sense::click())
            .selectable(false),
    );
    if resp.hovered() {
        let y = resp.rect.bottom() + 1.0;
        ui.painter().hline(
            resp.rect.left()..=resp.rect.right(),
            y,
            Stroke::new(1.0, pl.text_dim),
        );
    }
    resp.clicked()
}

/// Primary (accent-filled) button.
pub fn primary(ui: &mut Ui, text: &str) -> bool {
    let pl = p();
    ui.add(egui::Button::new(RichText::new(text).color(pl.on_accent)).fill(pl.accent))
        .clicked()
}

/// Round accent swatch; returns true when clicked.
pub fn swatch(ui: &mut Ui, color: Color32, selected: bool, tip: &str) -> bool {
    let pl = p();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::click());
    let t_sel = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("sel"), selected, 0.2);
    let t_hov = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.12);
    let painter = ui.painter();
    let r = 8.5 + 1.0 * t_hov;
    painter.circle_filled(rect.center(), r, color);
    painter.circle_stroke(rect.center(), r, Stroke::new(1.0, pl.line));
    if t_sel > 0.01 {
        let ring = mix(pl.bg, pl.text, t_sel);
        painter.circle_stroke(rect.center(), r + 1.0 + 2.0 * t_sel, Stroke::new(1.5, ring));
    }
    resp.on_hover_text(tip).clicked()
}
