//! Экспорт в SVG: снимок картинкой (пикселизация вжигается в неё, иначе её можно
//! снять в редакторе), остальные фигуры векторами. Фигурное выделение становится
//! маской, фигуры обрезаются по ней так же, как в PNG.

use crate::draw;
use crate::selection::IBox;
use crate::shapes::{self, Kind, Pt, Shape};
use ab_glyph::FontVec;
use std::fmt::Write as _;
use tiny_skia::{Mask, Pixmap};

const FONT_FAMILY: &str = "'Segoe UI', 'Helvetica Neue', Arial, sans-serif";

/// SVG размером с габарит выделения. Координаты фигур переводятся из снимка в SVG.
pub fn build(shot: &Pixmap, mask: &Mask, b: IBox, list: &[Shape], font: Option<&FontVec>, mm: Option<f32>) -> Result<String, String> {
    // Слой снимка: пикселизация вжигается, вне маски прозрачно.
    let mut base = shot.clone();
    for s in list.iter().filter(|s| matches!(s.kind, Kind::Pixelate(..))) {
        shapes::render(&mut base, s, shot, font, Some(mask), mm);
    }
    let md = mask.data();
    let fw = shot.width();
    let mut crop = Pixmap::new(b.w, b.h).ok_or("пустое выделение")?;
    let mut mpm = Pixmap::new(b.w, b.h).ok_or("пустое выделение")?;
    let mut shaped = false;
    {
        let (bd, cd, mdd) = (base.data(), crop.data_mut(), mpm.data_mut());
        for y in 0..b.h {
            for x in 0..b.w {
                let mi = ((b.y + y) * fw + b.x + x) as usize;
                let mv = md[mi];
                shaped |= mv != 255;
                let di = ((y * b.w + x) * 4) as usize;
                for c in 0..4 {
                    cd[di + c] = (bd[mi * 4 + c] as u32 * mv as u32 / 255) as u8;
                }
                // Маска SVG по яркости: белое видно, чёрное скрыто.
                mdd[di..di + 4].copy_from_slice(&[mv, mv, mv, 255]);
            }
        }
    }
    let png = crop.encode_png().map_err(|e| e.to_string())?;

    let mut o = String::new();
    let _ = writeln!(o, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        o,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#,
        w = b.w,
        h = b.h
    );
    let _ = writeln!(o, "<title>Frostshot</title>");
    if shaped {
        let mpng = mpm.encode_png().map_err(|e| e.to_string())?;
        let _ = writeln!(
            o,
            r#"<defs><mask id="selection" color-interpolation="sRGB" maskUnits="userSpaceOnUse" x="0" y="0" width="{w}" height="{h}"><image width="{w}" height="{h}" xlink:href="data:image/png;base64,{d}"/></mask></defs>"#,
            w = b.w,
            h = b.h,
            d = base64(&mpng)
        );
    }
    let _ = writeln!(
        o,
        r#"<image id="screenshot" width="{w}" height="{h}" xlink:href="data:image/png;base64,{d}"/>"#,
        w = b.w,
        h = b.h,
        d = base64(&png)
    );
    // Маска на внешней группе, сдвиг на внутренней: иначе маска сдвинется вместе с фигурами.
    let _ = writeln!(o, r#"<g id="annotations"{}>"#, if shaped { r#" mask="url(#selection)""# } else { "" });
    let _ = writeln!(o, r#"<g transform="translate({} {})">"#, -(b.x as i64), -(b.y as i64));
    for s in list {
        shape(&mut o, s, font, mm);
    }
    o.push_str("</g>\n</g>\n</svg>\n");
    Ok(o)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Число без лишних нулей: 12.5, 3, -0.25.
fn n(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            // Управляющие символы XML 1.0 не допускает.
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

fn stroke(c: [u8; 3], w: f32) -> String {
    format!(r#"fill="none" stroke="{}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round""#, hex(c), n(w))
}

fn points(pts: &[Pt]) -> String {
    let mut s = String::new();
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{},{}", n(p.0), n(p.1));
    }
    // Одна точка: короткий штрих, как в растре.
    if pts.len() == 1 {
        let _ = write!(s, " {},{}", n(pts[0].0 + 0.01), n(pts[0].1));
    }
    s
}

/// font-size для SVG. Растр (ab_glyph) задаёт размер как высоту строки
/// (ascent - descent), а SVG как кегль (em): переводим. Segoe UI: 2048 / 2724.
fn css_size(font: Option<&FontVec>, size: f32) -> f32 {
    use ab_glyph::Font as _;
    let k = font.and_then(|f| f.units_per_em().map(|em| em / f.height_unscaled())).unwrap_or(0.752);
    size * k
}

/// Метрики шрифта: верх строки -> базовая линия, межстрочный шаг, ширина строки.
/// Без шрифта: приближение под Segoe UI.
fn ascent(font: Option<&FontVec>, size: f32) -> f32 {
    font.map_or(size * 0.81, |f| draw::ascent(f, size))
}
fn line_height(font: Option<&FontVec>, size: f32) -> f32 {
    font.map_or(size * 1.33, |f| draw::line_height(f, size))
}
fn text_size(font: Option<&FontVec>, text: &str, size: f32) -> (f32, f32) {
    font.map_or_else(
        || {
            let lines = text.split('\n').count().max(1) as f32;
            let w = text.split('\n').map(|l| l.chars().count()).max().unwrap_or(0) as f32 * size * 0.55;
            (w, lines * size * 1.33)
        },
        |f| draw::text_size(f, text, size),
    )
}

/// Текст: верхний левый угол at, строки через tspan.
fn text(o: &mut String, at: Pt, t: &str, size: f32, c: [u8; 3], font: Option<&FontVec>, extra: &str) {
    let base = at.1 + ascent(font, size);
    let lh = line_height(font, size);
    let _ = write!(
        o,
        r#"<text x="{}" y="{}" font-family="{FONT_FAMILY}" font-size="{}" fill="{}" xml:space="preserve"{extra}>"#,
        n(at.0),
        n(base),
        n(css_size(font, size)),
        hex(c)
    );
    for (i, line) in t.split('\n').enumerate() {
        let _ = write!(o, r#"<tspan x="{}" y="{}">{}</tspan>"#, n(at.0), n(base + i as f32 * lh), esc(line));
    }
    o.push_str("</text>\n");
}

fn contrast(c: [u8; 3]) -> [u8; 3] {
    let luma = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    if luma > 160.0 { [0x11, 0x11, 0x11] } else { [0xff, 0xff, 0xff] }
}

fn ltrb(a: Pt, b: Pt) -> (f32, f32, f32, f32) {
    (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

fn shape(o: &mut String, s: &Shape, font: Option<&FontVec>, mm: Option<f32>) {
    let c = s.color;
    let w = s.width;
    match &s.kind {
        Kind::Pencil(p) if !p.is_empty() => {
            let _ = writeln!(o, r#"<polyline points="{}" {}/>"#, points(p), stroke(c, w));
        }
        Kind::Marker(p) if !p.is_empty() => {
            let _ = writeln!(o, r#"<polyline points="{}" {} stroke-opacity="0.4"/>"#, points(p), stroke(c, shapes::marker_width(w)));
        }
        Kind::Line(a, b) => {
            let _ = writeln!(o, r#"<line x1="{}" y1="{}" x2="{}" y2="{}" {}/>"#, n(a.0), n(a.1), n(b.0), n(b.1), stroke(c, w));
        }
        Kind::Arrow(a, b) => arrow(o, *a, *b, c, w),
        Kind::Rect(a, b) => {
            let (x, y, rw, rh) = ltrb(*a, *b);
            let _ = writeln!(o, r#"<rect x="{}" y="{}" width="{}" height="{}" {}/>"#, n(x), n(y), n(rw), n(rh), stroke(c, w));
        }
        Kind::FilledRect(a, b) => {
            let (x, y, rw, rh) = ltrb(*a, *b);
            let _ = writeln!(o, r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#, n(x), n(y), n(rw), n(rh), hex(c));
        }
        Kind::Ellipse(a, b) => {
            let (x, y, rw, rh) = ltrb(*a, *b);
            let _ = writeln!(
                o,
                r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" {}/>"#,
                n(x + rw / 2.0),
                n(y + rh / 2.0),
                n(rw / 2.0),
                n(rh / 2.0),
                stroke(c, w)
            );
        }
        Kind::Text { at, text: t } if !t.trim().is_empty() => text(o, *at, t, shapes::font_size(w), c, font, ""),
        Kind::Counter { at, n: num, tip } => counter(o, *at, *num, *tip, c, w, font),
        Kind::Ruler(a, b) => ruler(o, *a, *b, c, w, font, mm),
        // Пикселизация уже в слое снимка.
        _ => {}
    }
}

fn arrow(o: &mut String, a: Pt, b: Pt, c: [u8; 3], w: f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (w * 4.0).max(14.0).min(len);
    let half = head * 0.5;
    let base = (b.0 - ux * head, b.1 - uy * head);
    let _ = writeln!(o, "<g>");
    let _ = writeln!(
        o,
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" {}/>"#,
        n(a.0),
        n(a.1),
        n(base.0 + ux),
        n(base.1 + uy),
        stroke(c, w)
    );
    let _ = writeln!(
        o,
        r#"<polygon points="{},{} {},{} {},{}" fill="{}"/>"#,
        n(b.0),
        n(b.1),
        n(base.0 - uy * half),
        n(base.1 + ux * half),
        n(base.0 + uy * half),
        n(base.1 - ux * half),
        hex(c)
    );
    let _ = writeln!(o, "</g>");
}

fn counter(o: &mut String, at: Pt, num: u32, tip: Option<Pt>, c: [u8; 3], w: f32, font: Option<&FontVec>) {
    let r = shapes::counter_radius(w);
    let _ = writeln!(o, "<g>");
    if let Some(t) = tip {
        let (dx, dy) = (t.0 - at.0, t.1 - at.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len > r {
            let (ux, uy) = (dx / len, dy / len);
            let (px, py) = (-uy * r * 0.6, ux * r * 0.6);
            let _ = writeln!(
                o,
                r#"<polygon points="{},{} {},{} {},{}" fill="{}"/>"#,
                n(t.0),
                n(t.1),
                n(at.0 + px),
                n(at.1 + py),
                n(at.0 - px),
                n(at.1 - py),
                hex(c)
            );
        }
    }
    let _ = writeln!(o, r#"<circle cx="{}" cy="{}" r="{}" fill="{}"/>"#, n(at.0), n(at.1), n(r), hex(c));
    let label = num.to_string();
    let mut size = r * 1.5;
    let tw = text_size(font, &label, size).0;
    if tw > r * 1.5 {
        size *= r * 1.5 / tw;
    }
    // Базовая линия как в растре: центр по высоте цифр.
    let _ = writeln!(
        o,
        r#"<text x="{}" y="{}" font-family="{FONT_FAMILY}" font-size="{}" fill="{}" text-anchor="middle">{label}</text>"#,
        n(at.0),
        n(at.1 + size * 0.35),
        n(css_size(font, size)),
        hex(contrast(c))
    );
    let _ = writeln!(o, "</g>");
}

#[allow(clippy::too_many_arguments)]
fn ruler(o: &mut String, a: Pt, b: Pt, c: [u8; 3], w: f32, font: Option<&FontVec>, mm: Option<f32>) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let lw = (w * 0.5).max(1.5);
    let (px, py) = (-dy / len, dx / len);
    let t = 6.0 + w;
    let _ = writeln!(o, "<g>");
    let mut seg = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let _ = writeln!(o, r#"<line x1="{}" y1="{}" x2="{}" y2="{}" {}/>"#, n(x0), n(y0), n(x1), n(y1), stroke(c, lw));
    };
    seg(a.0, a.1, b.0, b.1);
    for e in [a, b] {
        seg(e.0 - px * t, e.1 - py * t, e.0 + px * t, e.1 + py * t);
    }
    let label = shapes::ruler_label(a, b, mm);
    let size = shapes::ruler_font(w);
    let (tw, th) = text_size(font, &label, size);
    let pad = 6.0;
    let off = t + th / 2.0 + pad;
    let (mx, my) = ((a.0 + b.0) / 2.0 + px * off, (a.1 + b.1) / 2.0 + py * off);
    let _ = writeln!(
        o,
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="5" fill="{}" fill-opacity="0.92"/>"#,
        n(mx - tw / 2.0 - pad),
        n(my - th / 2.0 - pad / 2.0),
        n(tw + 2.0 * pad),
        n(th + pad),
        hex(c)
    );
    text(o, (mx - tw / 2.0, my - th / 2.0), &label, size, contrast(c), font, "");
    let _ = writeln!(o, "</g>");
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(data.len().div_ceil(3) * 4);
    for ch in data.chunks(3) {
        let v = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        o.push(T[(v >> 18) as usize & 63] as char);
        o.push(T[(v >> 12) as usize & 63] as char);
        o.push(if ch.len() > 1 { T[(v >> 6) as usize & 63] as char } else { '=' });
        o.push(if ch.len() > 2 { T[v as usize & 63] as char } else { '=' });
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(esc("a<b & \"c\">\u{1}"), "a&lt;b &amp; &quot;c&quot;&gt;");
    }
}
