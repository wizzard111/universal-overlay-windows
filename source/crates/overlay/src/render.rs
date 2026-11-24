//! Tiny software renderer: premultiplied ARGB canvas, rectangles,
//! rounded rectangles, vertical spans and text (ab_glyph, lazily parsed
//! glyphs so a big system font costs only its file size in memory).
//! The overlay is a few hundred pixels, so CPU rendering is cheap and
//! needs no GPU context at all.

use ab_glyph::{Font, FontVec, PxScale};
use std::collections::HashMap;
use uo_common::config::Color;

/// Integer pixel rectangle inside a canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Pixels are premultiplied 0xAARRGGBB (matches a 32-bit top-down DIB).
pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
}

/// Source colour premultiplied by alpha·coverage, as 0..=255 integers.
#[inline]
fn premul(c: Color, cov: u32) -> (u32, u32, u32, u32) {
    let a = (c.a as u32 * cov + 127) / 255;
    let m = |v: u8| (v as u32 * a + 127) / 255;
    (a, m(c.r), m(c.g), m(c.b))
}

#[inline]
fn over(d: u32, (a, r, g, b): (u32, u32, u32, u32)) -> u32 {
    if a >= 255 {
        return (255 << 24) | (r << 16) | (g << 8) | b;
    }
    let inv = 255 - a;
    let ch = |s: u32, sh: u32| s + ((((d >> sh) & 0xFF) * inv + 127) / 255);
    (ch(a, 24).min(255) << 24)
        | (ch(r, 16).min(255) << 16)
        | (ch(g, 8).min(255) << 8)
        | ch(b, 0).min(255)
}

impl Canvas {
    pub fn new() -> Self {
        Self {
            w: 0,
            h: 0,
            px: Vec::new(),
        }
    }

    /// Become a copy of `other`, reusing this canvas's allocation.
    pub fn copy_from(&mut self, other: &Canvas) {
        self.w = other.w;
        self.h = other.h;
        self.px.clear();
        self.px.extend_from_slice(&other.px);
    }

    /// Copy the pixels of rectangle `r` from `other` (same size canvas).
    pub fn copy_rect_from(&mut self, other: &Canvas, r: PxRect) {
        for y in r.y..r.y + r.h {
            let i = (y * self.w + r.x) as usize;
            let n = r.w as usize;
            self.px[i..i + n].copy_from_slice(&other.px[i..i + n]);
        }
    }

    /// Multiply every (premultiplied) pixel by `k` — overall opacity.
    pub fn scale_alpha(&mut self, k: f32) {
        let r = PxRect {
            x: 0,
            y: 0,
            w: self.w,
            h: self.h,
        };
        self.scale_alpha_rect(k, r);
    }

    /// [`Canvas::scale_alpha`] limited to rectangle `r`.
    pub fn scale_alpha_rect(&mut self, k: f32, r: PxRect) {
        let m = (k.clamp(0.0, 1.0) * 256.0) as u32;
        let w = self.w;
        for p in (r.y..r.y + r.h).flat_map(|y| {
            let i = (y * w + r.x) as usize;
            i..i + r.w as usize
        }) {
            let p = &mut self.px[p];
            let v = *p;
            if v == 0 {
                continue;
            }
            let ch = |sh: u32| (((v >> sh) & 0xFF) * m) >> 8;
            *p = (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0);
        }
    }

    pub fn resize_clear(&mut self, w: i32, h: i32) {
        self.w = w.max(1);
        self.h = h.max(1);
        self.px.clear();
        self.px.resize((self.w * self.h) as usize, 0);
    }

    /// Blend with coverage 0..=255.
    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: Color, cov: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || cov == 0 {
            return;
        }
        let s = premul(c, cov);
        if s.0 == 0 {
            return;
        }
        let i = (y * self.w + x) as usize;
        self.px[i] = over(self.px[i], s);
    }

    /// Fill a horizontal run with full coverage.
    #[inline]
    fn hrun(&mut self, x0: i32, x1: i32, y: i32, s: (u32, u32, u32, u32)) {
        if y < 0 || y >= self.h {
            return;
        }
        let a = x0.max(0);
        let b = x1.min(self.w);
        if a >= b {
            return;
        }
        let row = (y * self.w) as usize;
        let packed = (s.0 << 24) | (s.1 << 16) | (s.2 << 8) | s.3;
        for p in &mut self.px[row + a as usize..row + b as usize] {
            // Fast path: the canvas starts cleared, so most fills land on
            // fully transparent pixels where "over" is a plain store.
            *p = if *p == 0 { packed } else { over(*p, s) };
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
        if c.a == 0 {
            return;
        }
        let s = premul(c, 255);
        for yy in y..y + h {
            self.hrun(x, x + w, yy, s);
        }
    }

    /// Anti-aliased rounded rectangle; only the corner pixels pay for AA.
    pub fn fill_round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Color) {
        if c.a == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = r.clamp(0.0, w.min(h) * 0.5);
        let full = premul(c, 255);
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = (x + w).ceil() as i32;
        let y1 = (y + h).ceil() as i32;
        let ri = r.ceil() as i32 + 1;
        let (cx, cy, hw, hh) = (x + w * 0.5, y + h * 0.5, w * 0.5, h * 0.5);
        for py in y0.max(0)..y1.min(self.h) {
            let edge_row = py < y0 + ri || py >= y1 - ri;
            if !edge_row {
                self.hrun(x0, x1, py, full);
                continue;
            }
            for px in x0.max(0)..x1.min(self.w) {
                let in_corner_band = px < x0 + ri || px >= x1 - ri;
                let fx = px as f32 + 0.5;
                let fy = py as f32 + 0.5;
                let qx = (fx - cx).abs() - (hw - r);
                let qy = (fy - cy).abs() - (hh - r);
                let outside = if in_corner_band {
                    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
                } else {
                    qy - r // straight top/bottom edge
                };
                let cov = ((0.5 - outside).clamp(0.0, 1.0) * 255.0) as u32;
                self.blend(px, py, c, cov);
            }
        }
    }

    /// Vertical span [y0, y1] (float, anti-aliased ends) in column x.
    pub fn vspan(&mut self, x: i32, y0: f32, y1: f32, c: Color) {
        if x < 0 || x >= self.w {
            return;
        }
        let (a, b) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        let ya = a.floor() as i32;
        let yb = b.ceil() as i32;
        for y in ya.max(0)..yb.min(self.h) {
            let top = (y as f32).max(a);
            let bot = ((y + 1) as f32).min(b);
            let cov = ((bot - top).clamp(0.0, 1.0) * 255.0) as u32;
            self.blend(x, y, c, cov);
        }
    }

    /// Dashed horizontal line.
    pub fn hline_dashed(&mut self, x: i32, y: i32, w: i32, dash: i32, c: Color) {
        let s = premul(c, 255);
        let mut xx = x;
        while xx < x + w {
            self.hrun(xx, (xx + dash).min(x + w), y, s);
            xx += dash * 2;
        }
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- text

struct Glyph {
    advance: f32,
    left: i32,
    /// Offset of the bitmap top from the baseline (negative = above).
    top: i32,
    w: usize,
    h: usize,
    cov: Vec<u8>,
}

pub struct Text {
    font: Option<FontVec>,
    cache: HashMap<(char, u32), Glyph>,
}

impl Text {
    pub fn new(font_bytes: Option<Vec<u8>>) -> Self {
        let font = font_bytes.and_then(|b| FontVec::try_from_vec(b).ok());
        Self {
            font,
            cache: HashMap::new(),
        }
    }

    pub fn has_font(&self) -> bool {
        self.font.is_some()
    }

    /// Convert an em size in px (like CSS font-size) to ab_glyph's scale.
    fn px_scale(font: &FontVec, size: f32) -> PxScale {
        let upem = font.units_per_em().unwrap_or(1000.0);
        PxScale::from(size * font.height_unscaled() / upem)
    }

    fn glyph(&mut self, ch: char, size: f32) -> Option<&Glyph> {
        let font = self.font.as_ref()?;
        let key = (ch, (size * 64.0) as u32);
        if self.cache.len() > 2048 {
            self.cache.clear();
        }
        Some(self.cache.entry(key).or_insert_with(|| {
            let scale = Self::px_scale(font, size);
            let id = font.glyph_id(ch);
            let sf = font.as_scaled(scale);
            let advance = ab_glyph::ScaleFont::h_advance(&sf, id);
            let g = id.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
            match font.outline_glyph(g) {
                Some(og) => {
                    let b = og.px_bounds();
                    let w = b.width() as usize;
                    let h = b.height() as usize;
                    let mut cov = vec![0u8; w * h];
                    og.draw(|x, y, c| {
                        let i = y as usize * w + x as usize;
                        if i < cov.len() {
                            cov[i] = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                        }
                    });
                    Glyph {
                        advance,
                        left: b.min.x as i32,
                        top: b.min.y as i32,
                        w,
                        h,
                        cov,
                    }
                }
                None => Glyph {
                    advance,
                    left: 0,
                    top: 0,
                    w: 0,
                    h: 0,
                    cov: Vec::new(),
                },
            }
        }))
    }

    /// (ascent, descent) in px; descent is positive.
    pub fn line_metrics(&self, size: f32) -> (f32, f32) {
        match self.font.as_ref() {
            Some(f) => {
                let upem = f.units_per_em().unwrap_or(1000.0);
                (
                    f.ascent_unscaled() * size / upem,
                    -f.descent_unscaled() * size / upem,
                )
            }
            None => (size * 0.8, size * 0.2),
        }
    }

    /// Height of capital letters.
    pub fn cap_height(&mut self, size: f32) -> f32 {
        match self.glyph('H', size) {
            Some(g) => g.h as f32,
            None => size * 0.7,
        }
    }

    pub fn measure(&mut self, s: &str, size: f32) -> f32 {
        s.chars()
            .filter_map(|ch| self.glyph(ch, size).map(|g| g.advance.round()))
            .sum()
    }

    /// Draw `s` with its baseline at `baseline`. Returns the advance.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        cv: &mut Canvas,
        x: f32,
        baseline: f32,
        s: &str,
        size: f32,
        c: Color,
        shadow: bool,
    ) -> f32 {
        if shadow {
            let sc = Color::rgba(0, 0, 0, (c.a as u32 * 200 / 255) as u8);
            self.draw_raw(cv, x + 1.0, baseline + 1.0, s, size, sc);
        }
        self.draw_raw(cv, x, baseline, s, size, c)
    }

    fn draw_raw(
        &mut self,
        cv: &mut Canvas,
        x: f32,
        baseline: f32,
        s: &str,
        size: f32,
        c: Color,
    ) -> f32 {
        let start = x.round();
        let mut pen = start;
        let base = baseline.round() as i32;
        for ch in s.chars() {
            let Some(g) = self.glyph(ch, size) else {
                continue;
            };
            let gx = pen as i32 + g.left;
            let gy = base + g.top;
            for row in 0..g.h {
                for col in 0..g.w {
                    let v = g.cov[row * g.w + col];
                    if v != 0 {
                        cv.blend(gx + col as i32, gy + row as i32, c, v as u32);
                    }
                }
            }
            pen += g.advance.round();
        }
        pen - start
    }
}
