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
    Text,
    Pixelate,
}

impl Tool {
    pub const ALL: [Tool; 9] = [
        Tool::SelectRect,
        Tool::SelectLasso,
        Tool::Pencil,
        Tool::Marker,
        Tool::Line,
        Tool::Arrow,
        Tool::Rect,
        Tool::Text,
        Tool::Pixelate,
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
            Tool::Rect => "Прямоугольник (5)",
            Tool::Text => "Текст (6)",
            Tool::Pixelate => "Пикселизация (7)",
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

pub fn marker_width(width: f32) -> f32 {
    (width * 3.0).max(12.0)
}

impl Shape {
    /// Фигура достаточно велика, чтобы её сохранить.
    pub fn is_meaningful(&self) -> bool {
        let far = |a: Pt, b: Pt| (a.0 - b.0).abs() + (a.1 - b.1).abs() >= 3.0;
        match &self.kind {
            Kind::Pencil(p) | Kind::Marker(p) => !p.is_empty(),
            Kind::Line(a, b) | Kind::Arrow(a, b) | Kind::Rect(a, b) | Kind::Pixelate(a, b) => far(*a, *b),
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
        _ => {}
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
    let block = (8.0 + w * 2.0).round() as i32;
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
