//! Экспорт в SVG: снимок картинкой, фигуры векторами. Пикселизация и закрашенный
//! прямоугольник вжигаются в картинку (иначе их можно снять в редакторе и увидеть,
//! что под ними), вместе с фигурами, которые лежат под ними: порядок наложения как
//! в PNG. Фигурное выделение становится маской, фигуры обрезаются по ней так же,
//! как в PNG. Фигуры вне выделения по умолчанию не попадают в файл.

use crate::draw;
use crate::selection::IBox;
use crate::shapes::{self, Kind, Pt, Shape};
use ab_glyph::FontVec;
use std::fmt::Write as _;
use tiny_skia::{Mask, Pixmap};

const FONT_FAMILY: &str = "'Segoe UI', 'Helvetica Neue', Arial, sans-serif";

/// SVG размером с габарит выделения. Координаты фигур переводятся из снимка в SVG.
/// keep_outside: сохранить и фигуры целиком вне выделения (невидимые, за краем листа).
#[allow(clippy::too_many_arguments)]
pub fn build(
    shot: &Pixmap,
    mask: &Mask,
    b: IBox,
    list: &[Shape],
    font: Option<&FontVec>,
    mm: Option<f32>,
    keep_outside: bool,
) -> Result<String, String> {
    // Слой снимка: вжигаемые фигуры в порядке списка, вне маски прозрачно.
    let burn = burned(list, font, mm);
    let mut base = shot.clone();
    for (s, _) in list.iter().zip(&burn).filter(|(_, b)| **b) {
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
    for (s, &bn) in list.iter().zip(&burn) {
        if bn || (!keep_outside && !visible(mask, b, s.bounds(font, mm))) {
            continue;
        }
        shape(&mut o, s, font, mm);
    }
    o.push_str("</g>\n</g>\n</svg>\n");
    Ok(o)
}

type Bounds = (f32, f32, f32, f32);

fn cross(a: Bounds, b: Bounds) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// Какие фигуры вжечь в картинку. Пикселизация и закрашенный прямоугольник всегда:
/// они прячут данные. И всё, что раньше по списку и пересекается с вжигаемым:
/// в PNG оно под ним, и в SVG не должно оказаться сверху.
fn burned(list: &[Shape], font: Option<&FontVec>, mm: Option<f32>) -> Vec<bool> {
    let mut out = vec![false; list.len()];
    let mut over: Vec<Bounds> = Vec::new();
    for i in (0..list.len()).rev() {
        let bb = list[i].bounds(font, mm);
        let hides = matches!(list[i].kind, Kind::Pixelate(..) | Kind::FilledRect(..));
        if hides || over.iter().any(|q| cross(bb, *q)) {
            out[i] = true;
            over.push(bb);
        }
    }
    out
}

/// Видна ли хоть часть габарита bb (координаты снимка) в выделении.
fn visible(mask: &Mask, b: IBox, bb: Bounds) -> bool {
    let l = bb.0.floor().max(b.x as f32);
    let t = bb.1.floor().max(b.y as f32);
    let r = bb.2.ceil().min((b.x + b.w) as f32);
    let bt = bb.3.ceil().min((b.y + b.h) as f32);
    if !(l < r && t < bt) {
        return false;
    }
    let (l, t, r, bt) = (l as usize, t as usize, r as usize, bt as usize);
    let fw = mask.width() as usize;
    let md = mask.data();
    (t..bt).any(|y| md[y * fw + l..y * fw + r].iter().any(|&v| v > 0))
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
            c if xml_char(c) => o.push(c),
            // Символы вне XML 1.0 (управляющие, U+FFFE, U+FFFF) ломают файл: пропускаем.
            _ => {}
        }
    }
    o
}

/// Допустимый символ XML 1.0 (производная Char).
fn xml_char(c: char) -> bool {
    matches!(c as u32, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
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
        // Нулевая ширина или высота: SVG такую рамку и эллипс не рисует, а PNG рисует
        // полоску толщиной в линию. Пишем линией.
        // Сравнение после округления до сотых, как пишутся числа (n): 0,004 тоже ноль.
        Kind::Rect(a, b) | Kind::Ellipse(a, b) if ltrb(*a, *b).2 < 0.02 || ltrb(*a, *b).3 < 0.02 => {
            let (x, y, rw, rh) = ltrb(*a, *b);
            let _ = writeln!(o, r#"<line x1="{}" y1="{}" x2="{}" y2="{}" {}/>"#, n(x), n(y), n(x + rw), n(y + rh), stroke(c, w));
        }
        Kind::Rect(a, b) => {
            let (x, y, rw, rh) = ltrb(*a, *b);
            let _ = writeln!(o, r#"<rect x="{}" y="{}" width="{}" height="{}" {}/>"#, n(x), n(y), n(rw), n(rh), stroke(c, w));
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
        // Пикселизация и закрашенный прямоугольник всегда в слое снимка (burned).
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
    let head = shapes::arrow_head(w).min(len);
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
    let lw = shapes::ruler_lw(w);
    let (px, py) = (-dy / len, dx / len);
    let t = shapes::ruler_tick(w);
    let _ = writeln!(o, "<g>");
    let mut seg = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let _ = writeln!(o, r#"<line x1="{}" y1="{}" x2="{}" y2="{}" {}/>"#, n(x0), n(y0), n(x1), n(y1), stroke(c, lw));
    };
    seg(a.0, a.1, b.0, b.1);
    for e in [a, b] {
        seg(e.0 - px * t, e.1 - py * t, e.0 + px * t, e.1 + py * t);
    }
    // Плашка там же, где в PNG (shapes::ruler_plate); без шрифта: приближение.
    let pl = font.and_then(|f| shapes::ruler_plate(a, b, w, f, mm)).unwrap_or_else(|| {
        let text = shapes::ruler_label(a, b, mm);
        let size = shapes::ruler_font(w);
        let (tw, th) = text_size(None, &text, size);
        let pad = 6.0;
        let off = t + th / 2.0 + pad;
        let (mx, my) = ((a.0 + b.0) / 2.0 + px * off, (a.1 + b.1) / 2.0 + py * off);
        shapes::Plate { text, size, x: mx - tw / 2.0 - pad, y: my - th / 2.0 - pad / 2.0, w: tw + 2.0 * pad, h: th + pad, pad }
    });
    let _ = writeln!(
        o,
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="5" fill="{}" fill-opacity="0.92"/>"#,
        n(pl.x),
        n(pl.y),
        n(pl.w),
        n(pl.h),
        hex(c)
    );
    text(o, (pl.x + pl.pad, pl.y + pl.pad / 2.0), &pl.text, pl.size, contrast(c), font, "");
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

    /// Снимок 200x100, выделение: левая половина (100x100).
    fn scene(shapes: &[Shape], keep_outside: bool) -> String {
        let shot = Pixmap::new(200, 100).unwrap();
        let mut mask = Mask::new(200, 100).unwrap();
        for y in 0..100 {
            for x in 0..100 {
                mask.data_mut()[y * 200 + x] = 255;
            }
        }
        build(&shot, &mask, IBox { x: 0, y: 0, w: 100, h: 100 }, shapes, None, None, keep_outside).unwrap()
    }

    fn sh(kind: Kind) -> Shape {
        Shape { kind, color: [255, 0, 0], width: 4.0 }
    }

    #[test]
    fn text_under_pixelation_is_burned_not_vector() {
        let svg = scene(&[sh(Kind::Text { at: (10.0, 10.0), text: "SECRET".into() }), sh(Kind::Pixelate((0.0, 0.0), (90.0, 60.0)))], false);
        assert!(!svg.contains("SECRET"), "hidden text leaked: {svg}");
    }

    #[test]
    fn filled_rect_is_burned_with_what_is_under_it() {
        let svg = scene(&[sh(Kind::Text { at: (10.0, 10.0), text: "IBAN".into() }), sh(Kind::FilledRect((0.0, 0.0), (90.0, 60.0)))], false);
        assert!(!svg.contains("IBAN"));
        assert!(!svg.contains("<rect"), "filled rect must not be an editable vector");
    }

    #[test]
    fn shapes_above_or_beside_burned_stay_vectors() {
        let svg = scene(
            &[sh(Kind::Pixelate((0.0, 0.0), (40.0, 40.0))), sh(Kind::Text { at: (10.0, 10.0), text: "note".into() }), sh(Kind::Line((50.0, 80.0), (90.0, 80.0)))],
            false,
        );
        assert!(svg.contains("note"), "text drawn after pixelation stays on top as text");
        assert!(svg.contains("<line"));
    }

    #[test]
    fn shapes_outside_selection_follow_the_setting() {
        let list = [sh(Kind::Text { at: (150.0, 10.0), text: "outside".into() }), sh(Kind::Line((10.0, 50.0), (60.0, 50.0)))];
        let svg = scene(&list, false);
        assert!(!svg.contains("outside"));
        assert!(svg.contains("<line"));
        assert!(scene(&list, true).contains("outside"));
    }

    #[test]
    fn flat_rect_and_ellipse_become_lines() {
        let svg = scene(&[sh(Kind::Rect((10.0, 50.0), (80.0, 50.0))), sh(Kind::Ellipse((10.0, 20.0), (10.0, 70.0)))], false);
        assert_eq!(svg.matches("<line").count(), 2, "{svg}");
        assert!(!svg.contains("<rect") && !svg.contains("<ellipse"));
    }

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
        assert_eq!(esc("a\u{FFFE}b\u{FFFF}c\u{10000}"), "abc\u{10000}");
    }
}
