//! Низкоуровневые помощники рисования: пиксели, текст, примитивы tiny-skia.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use tiny_skia::{
    FillRule, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

pub type Rgb = [u8; 3];

pub fn rgb(hex: u32) -> Rgb {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

pub fn paint(c: Rgb, alpha: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], (alpha.clamp(0.0, 1.0) * 255.0) as u8);
    p.anti_alias = true;
    p
}

pub fn mask_at(m: &Mask, x: i32, y: i32) -> f32 {
    if x < 0 || y < 0 || x >= m.width() as i32 || y >= m.height() as i32 {
        return 0.0;
    }
    m.data()[(y as u32 * m.width() + x as u32) as usize] as f32 / 255.0
}

/// Source-over одного пикселя (данные premultiplied).
pub fn blend(pm: &mut Pixmap, x: i32, y: i32, c: Rgb, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 || x >= pm.width() as i32 || y >= pm.height() as i32 {
        return;
    }
    let a = a.min(1.0);
    let ia = 1.0 - a;
    let i = ((y as u32 * pm.width() + x as u32) * 4) as usize;
    let d = pm.data_mut();
    d[i] = (c[0] as f32 * a + d[i] as f32 * ia + 0.5) as u8;
    d[i + 1] = (c[1] as f32 * a + d[i + 1] as f32 * ia + 0.5) as u8;
    d[i + 2] = (c[2] as f32 * a + d[i + 2] as f32 * ia + 0.5) as u8;
    d[i + 3] = (255.0 * a + d[i + 3] as f32 * ia + 0.5) as u8;
}

pub fn load_font() -> Option<FontVec> {
    for path in crate::platform::font_candidates() {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(f) = FontVec::try_from_vec_and_index(bytes, 0) {
                log::info!("font: {path}");
                return Some(f);
            }
        }
    }
    log::warn!("no system font found, text disabled");
    None
}

pub fn line_height(font: &FontVec, size: f32) -> f32 {
    let s = font.as_scaled(PxScale::from(size));
    s.height() + s.line_gap()
}

/// Размер многострочного текста (ширина, высота).
pub fn text_size(font: &FontVec, text: &str, size: f32) -> (f32, f32) {
    let s = font.as_scaled(PxScale::from(size));
    let mut w: f32 = 0.0;
    let mut lines = 0;
    for line in text.split('\n') {
        lines += 1;
        let mut lw = 0.0;
        let mut prev = None;
        for ch in line.chars() {
            let g = s.glyph_id(ch);
            if let Some(p) = prev {
                lw += s.kern(p, g);
            }
            lw += s.h_advance(g);
            prev = Some(g);
        }
        w = w.max(lw);
    }
    (w, lines as f32 * line_height(font, size))
}

/// Текст с левым верхним углом в (x, y). clip: маска обрезки (выделение).
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    pm: &mut Pixmap,
    font: &FontVec,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    c: Rgb,
    alpha: f32,
    clip: Option<&Mask>,
) {
    let s = font.as_scaled(PxScale::from(size));
    let lh = line_height(font, size);
    for (li, line) in text.split('\n').enumerate() {
        let mut caret = point(x, y + s.ascent() + li as f32 * lh);
        let mut prev = None;
        for ch in line.chars() {
            let gid = s.glyph_id(ch);
            if let Some(p) = prev {
                caret.x += s.kern(p, gid);
            }
            let glyph = gid.with_scale_and_position(size, caret);
            caret.x += s.h_advance(gid);
            prev = Some(gid);
            if let Some(og) = font.outline_glyph(glyph) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| {
                    let px = b.min.x as i32 + gx as i32;
                    let py = b.min.y as i32 + gy as i32;
                    let m = clip.map_or(1.0, |m| mask_at(m, px, py));
                    blend(pm, px, py, c, cov * alpha * m);
                });
            }
        }
    }
}

pub fn fill_rect(pm: &mut Pixmap, r: Rect, c: Rgb, alpha: f32, clip: Option<&Mask>) {
    let mut p = paint(c, alpha);
    p.anti_alias = false;
    pm.fill_rect(r, &p, Transform::identity(), clip);
}

pub fn rounded_rect(r: Rect, rad: f32) -> Option<Path> {
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0);
    let (l, t, rr, b) = (r.left(), r.top(), r.right(), r.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(l + rad, t);
    pb.line_to(rr - rad, t);
    pb.quad_to(rr, t, rr, t + rad);
    pb.line_to(rr, b - rad);
    pb.quad_to(rr, b, rr - rad, b);
    pb.line_to(l + rad, b);
    pb.quad_to(l, b, l, b - rad);
    pb.line_to(l, t + rad);
    pb.quad_to(l, t, l + rad, t);
    pb.close();
    pb.finish()
}

pub fn fill_rounded(pm: &mut Pixmap, r: Rect, rad: f32, c: Rgb, alpha: f32) {
    if let Some(path) = rounded_rect(r, rad) {
        pm.fill_path(&path, &paint(c, alpha), FillRule::Winding, Transform::identity(), None);
    }
}

pub fn stroke_path(pm: &mut Pixmap, path: &Path, c: Rgb, alpha: f32, width: f32, clip: Option<&Mask>) {
    let stroke = Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pm.stroke_path(path, &paint(c, alpha), &stroke, Transform::identity(), clip);
}

pub fn line(pm: &mut Pixmap, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb, alpha: f32, w: f32, clip: Option<&Mask>) {
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y0);
    pb.line_to(x1, y1);
    if let Some(p) = pb.finish() {
        stroke_path(pm, &p, c, alpha, w, clip);
    }
}

pub fn circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, c: Rgb, alpha: f32, fill: bool, w: f32) {
    if let Some(p) = PathBuilder::from_circle(cx, cy, r) {
        if fill {
            pm.fill_path(&p, &paint(c, alpha), FillRule::Winding, Transform::identity(), None);
        } else {
            stroke_path(pm, &p, c, alpha, w, None);
        }
    }
}

pub fn rect_ltrb(l: f32, t: f32, r: f32, b: f32) -> Option<Rect> {
    Rect::from_ltrb(l.min(r), t.min(b), l.max(r), t.max(b))
}
