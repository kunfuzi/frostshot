//! Фигуры разметки. Координаты в пикселях снимка монитора (инвариант 8):
//! одна функция рендера и для экрана, и для итогового файла.

use crate::draw::{self, Rgb};
use ab_glyph::FontVec;
use serde::{Deserialize, Serialize};
use tiny_skia::{FillRule, Mask, PathBuilder, Pixmap, Transform};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    SelectRect,
    SelectLasso,
    Pencil,
    Marker,
    Line,
    Arrow,
    Rect,
    FilledRect,
    Ellipse,
    Text,
    Counter,
    Pixelate,
    Ruler,
}

impl Tool {
    pub const ALL: [Tool; 13] = [
        Tool::SelectRect,
        Tool::SelectLasso,
        Tool::Pencil,
        Tool::Marker,
        Tool::Line,
        Tool::Arrow,
        Tool::Rect,
        Tool::FilledRect,
        Tool::Ellipse,
        Tool::Text,
        Tool::Counter,
        Tool::Pixelate,
        Tool::Ruler,
    ];

    pub fn is_selection(self) -> bool {
        matches!(self, Tool::SelectRect | Tool::SelectLasso)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::SelectRect => "Рамка (V) · Shift: добавить, Alt: вычесть",
            Tool::SelectLasso => "Лассо (L) · Shift: добавить, Alt: вычесть",
            Tool::Pencil => "Карандаш (1)",
            Tool::Marker => "Маркер (2)",
            Tool::Line => "Линия (3)",
            Tool::Arrow => "Стрелка (4)",
            Tool::Rect => "Прямоугольник (5, Shift: квадрат)",
            Tool::FilledRect => "Закрашенный прямоугольник (8)",
            Tool::Ellipse => "Эллипс (9, Shift: круг)",
            Tool::Counter => "Счётчик (0): тяни от номера к цели, с Shift от цели",
            Tool::Text => "Текст (6)",
            Tool::Pixelate => "Пикселизация (7)",
            Tool::Ruler => "Линейка (R, Shift: 45°)",
        }
    }

}

pub type Pt = (f32, f32);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Kind {
    Pencil(Vec<Pt>),
    Marker(Vec<Pt>),
    Line(Pt, Pt),
    Arrow(Pt, Pt),
    Rect(Pt, Pt),
    Text { at: Pt, text: String },
    Pixelate(Pt, Pt),
    FilledRect(Pt, Pt),
    Ellipse(Pt, Pt),
    /// tip: точка, на которую указывает выноска-клин (None: просто кружок).
    Counter {
        at: Pt,
        n: u32,
        #[serde(default)]
        tip: Option<Pt>,
    },
    Ruler(Pt, Pt),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shape {
    pub kind: Kind,
    pub color: Rgb,
    pub width: f32,
}

pub fn font_size(width: f32) -> f32 {
    14.0 + width * 3.0
}

/// Радиус кружка счётчика.
pub fn counter_radius(width: f32) -> f32 {
    11.0 + width * 1.5
}

pub fn marker_width(width: f32) -> f32 {
    (width * 3.0).max(12.0)
}

impl Shape {
    /// Фигура достаточно велика, чтобы её сохранить.
    pub fn is_meaningful(&self) -> bool {
        let far = |a: Pt, b: Pt| (a.0 - b.0).abs() + (a.1 - b.1).abs() >= 3.0;
        match &self.kind {
            Kind::Pencil(p) | Kind::Marker(p) => !p.is_empty(),
            Kind::Line(a, b) | Kind::Arrow(a, b) | Kind::Rect(a, b) | Kind::Pixelate(a, b) | Kind::FilledRect(a, b) | Kind::Ellipse(a, b) => {
                far(*a, *b)
            }
            Kind::Counter { .. } => true,
            Kind::Ruler(a, b) => far(*a, *b),
            Kind::Text { text, .. } => !text.trim().is_empty(),
        }
    }
}

fn polyline(pts: &[Pt]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    if pts.len() == 1 {
        pb.line_to(pts[0].0 + 0.01, pts[0].1);
    }
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    pb.finish()
}

pub fn render(pm: &mut Pixmap, s: &Shape, src: &Pixmap, font: Option<&FontVec>, clip: Option<&Mask>) {
    let c = s.color;
    match &s.kind {
        Kind::Pencil(pts) if !pts.is_empty() => {
            if let Some(p) = polyline(pts) {
                draw::stroke_path(pm, &p, c, 1.0, s.width, clip);
            }
        }
        Kind::Marker(pts) if !pts.is_empty() => {
            if let Some(p) = polyline(pts) {
                draw::stroke_path(pm, &p, c, 0.4, marker_width(s.width), clip);
            }
        }
        Kind::Line(a, b) => draw::line(pm, a.0, a.1, b.0, b.1, c, 1.0, s.width, clip),
        Kind::Arrow(a, b) => arrow(pm, *a, *b, c, s.width, clip),
        Kind::Rect(a, b) => {
            if let Some(r) = draw::rect_ltrb(a.0, a.1, b.0, b.1) {
                let p = PathBuilder::from_rect(r);
                draw::stroke_path(pm, &p, c, 1.0, s.width, clip);
            }
        }
        Kind::Text { at, text } => {
            if let Some(f) = font {
                draw::draw_text(pm, f, text, at.0, at.1, font_size(s.width), c, 1.0, clip);
            }
        }
        Kind::Pixelate(a, b) => pixelate(pm, src, *a, *b, s.width, clip),
        Kind::FilledRect(a, b) => {
            if let Some(r) = draw::rect_ltrb(a.0, a.1, b.0, b.1) {
                let p = PathBuilder::from_rect(r);
                pm.fill_path(&p, &draw::paint(c, 1.0), FillRule::Winding, Transform::identity(), clip);
            }
        }
        Kind::Ellipse(a, b) => {
            if let Some(p) = draw::rect_ltrb(a.0, a.1, b.0, b.1).and_then(PathBuilder::from_oval) {
                draw::stroke_path(pm, &p, c, 1.0, s.width, clip);
            }
        }
        Kind::Counter { at, n, tip } => counter(pm, *at, *n, *tip, c, s.width, font, clip),
        Kind::Ruler(a, b) => ruler(pm, *a, *b, c, s.width, font, clip),
        _ => {}
    }
}

/// Кружок с номером и необязательной выноской-клином к точке tip.
/// Цифра белая или чёрная в зависимости от яркости цвета.
#[allow(clippy::too_many_arguments)]
fn counter(pm: &mut Pixmap, at: Pt, n: u32, tip: Option<Pt>, c: Rgb, w: f32, font: Option<&FontVec>, clip: Option<&Mask>) {
    let r = counter_radius(w);
    if let Some(t) = tip {
        let (dx, dy) = (t.0 - at.0, t.1 - at.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len > r {
            // Основание клина внутри кружка, остриё в цели.
            let (ux, uy) = (dx / len, dy / len);
            let (px, py) = (-uy * r * 0.6, ux * r * 0.6);
            let mut pb = PathBuilder::new();
            pb.move_to(t.0, t.1);
            pb.line_to(at.0 + px, at.1 + py);
            pb.line_to(at.0 - px, at.1 - py);
            pb.close();
            if let Some(p) = pb.finish() {
                pm.fill_path(&p, &draw::paint(c, 1.0), FillRule::Winding, Transform::identity(), clip);
            }
        }
    }
    if let Some(p) = PathBuilder::from_circle(at.0, at.1, r) {
        pm.fill_path(&p, &draw::paint(c, 1.0), FillRule::Winding, Transform::identity(), clip);
    }
    if let Some(f) = font {
        let luma = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
        let fg = if luma > 160.0 { [0x11, 0x11, 0x11] } else { [0xff, 0xff, 0xff] };
        let text = n.to_string();
        // Крупная цифра; многозначный номер ужимается, чтобы остаться внутри кружка.
        let mut size = r * 1.5;
        let tw = draw::text_size(f, &text, size).0;
        if tw > r * 1.5 {
            size *= r * 1.5 / tw;
        }
        let tw = draw::text_size(f, &text, size).0;
        // Центр по высоте цифр (≈0.7 кегля), а не по всей строке.
        let y = at.1 + size * 0.35 - draw::ascent(f, size);
        draw::draw_text(pm, f, &text, at.0 - tw / 2.0, y, size, fg, 1.0, clip);
    }
}

/// Подпись длины: «240 px», для наклонной линии ещё проекции «(200 × 133)».
pub fn ruler_label(a: Pt, b: Pt) -> String {
    let (dx, dy) = ((b.0 - a.0).abs().round(), (b.1 - a.1).abs().round());
    let len = (dx * dx + dy * dy).sqrt().round();
    if dx > 0.0 && dy > 0.0 { format!("{len} px ({dx} × {dy})") } else { format!("{len} px") }
}

/// Линейка: тонкая линия, засечки на концах, подпись длины на плашке у середины.
fn ruler(pm: &mut Pixmap, a: Pt, b: Pt, c: Rgb, w: f32, font: Option<&FontVec>, clip: Option<&Mask>) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let lw = (w * 0.5).max(1.5);
    let (px, py) = (-dy / len, dx / len);
    let t = 6.0 + w;
    draw::line(pm, a.0, a.1, b.0, b.1, c, 1.0, lw, clip);
    for e in [a, b] {
        draw::line(pm, e.0 - px * t, e.1 - py * t, e.0 + px * t, e.1 + py * t, c, 1.0, lw, clip);
    }
    if let Some(f) = font {
        let text = ruler_label(a, b);
        let size = (12.0 + w).max(13.0);
        let (tw, th) = draw::text_size(f, &text, size);
        let pad = 4.0;
        // Плашка сбоку от середины линии, чтобы не закрывать саму линию.
        let off = t + th / 2.0 + pad;
        let (mx, my) = ((a.0 + b.0) / 2.0 + px * off, (a.1 + b.1) / 2.0 + py * off);
        if let Some(r) = tiny_skia::Rect::from_xywh(mx - tw / 2.0 - pad, my - th / 2.0 - pad / 2.0, tw + 2.0 * pad, th + pad) {
            if let Some(p) = draw::rounded_rect(r, 4.0) {
                pm.fill_path(&p, &draw::paint(c, 0.92), FillRule::Winding, Transform::identity(), clip);
            }
            let luma = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
            let fg = if luma > 160.0 { [0x11, 0x11, 0x11] } else { [0xff, 0xff, 0xff] };
            draw::draw_text(pm, f, &text, mx - tw / 2.0, my - th / 2.0, size, fg, 1.0, clip);
        }
    }
}

fn arrow(pm: &mut Pixmap, a: Pt, b: Pt, c: Rgb, w: f32, clip: Option<&Mask>) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (w * 4.0).max(14.0).min(len);
    let half = head * 0.5;
    let base = (b.0 - ux * head, b.1 - uy * head);
    draw::line(pm, a.0, a.1, base.0 + ux * 1.0, base.1 + uy * 1.0, c, 1.0, w, clip);
    let mut pb = PathBuilder::new();
    pb.move_to(b.0, b.1);
    pb.line_to(base.0 - uy * half, base.1 + ux * half);
    pb.line_to(base.0 + uy * half, base.1 - ux * half);
    pb.close();
    if let Some(p) = pb.finish() {
        pm.fill_path(&p, &draw::paint(c, 1.0), FillRule::Winding, Transform::identity(), clip);
    }
}

fn pixelate(pm: &mut Pixmap, src: &Pixmap, a: Pt, b: Pt, w: f32, clip: Option<&Mask>) {
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    let x0 = (a.0.min(b.0).floor() as i32).clamp(0, sw);
    let y0 = (a.1.min(b.1).floor() as i32).clamp(0, sh);
    let x1 = (a.0.max(b.0).ceil() as i32).clamp(0, sw);
    let y1 = (a.1.max(b.1).ceil() as i32).clamp(0, sh);
    let block = ((8.0 + w * 2.0).round() as i32).max(2);
    let d = src.data();
    let mut by = y0;
    while by < y1 {
        let bh = block.min(y1 - by);
        let mut bx = x0;
        while bx < x1 {
            let bw = block.min(x1 - bx);
            let mut acc = [0u64; 3];
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let i = ((y * sw + x) * 4) as usize;
                    acc[0] += d[i] as u64;
                    acc[1] += d[i + 1] as u64;
                    acc[2] += d[i + 2] as u64;
                }
            }
            let n = (bw * bh) as u64;
            let col = [(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8];
            if let Some(r) = tiny_skia::Rect::from_xywh(bx as f32, by as f32, bw as f32, bh as f32) {
                draw::fill_rect(pm, r, col, 1.0, clip);
            }
            bx += block;
        }
        by += block;
    }
}
