//! Формат проекта `.frost`: сигнатура, длина и JSON-заголовок, затем PNG снимка.
//!
//! Снимок хранится без пикселизации: разметка остаётся редактируемой.
//! Поэтому проект не для отправки, отправлять нужно экспорт в PNG.

use crate::selection::{SelOp, SelShape};
use crate::shapes::Shape;
use serde::{Deserialize, Serialize};
use tiny_skia::Rect;

const MAGIC: &[u8; 10] = b"FROSTSHOT\0";
pub const VERSION: u32 = 1;
pub const EXT: &str = "frost";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SelDto {
    Rect { x: f32, y: f32, w: f32, h: f32 },
    Poly { points: Vec<(f32, f32)> },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpDto {
    pub add: bool,
    pub shape: SelDto,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Header {
    pub version: u32,
    pub app_version: String,
    pub width: u32,
    pub height: u32,
    pub png_len: usize,
    pub selection: Vec<OpDto>,
    pub shapes: Vec<Shape>,
    pub color: u32,
    pub line_width: f32,
    /// Миллиметров на пиксель у монитора снимка (для линейки).
    #[serde(default)]
    pub mm_per_px: Option<f32>,
}

pub fn op_to_dto(op: &SelOp) -> OpDto {
    let shape = match &op.shape {
        SelShape::Rect(r) => SelDto::Rect { x: r.x(), y: r.y(), w: r.width(), h: r.height() },
        SelShape::Poly(p) => SelDto::Poly { points: p.clone() },
    };
    OpDto { add: op.add, shape }
}

pub fn dto_to_op(d: &OpDto) -> Option<SelOp> {
    let shape = match &d.shape {
        SelDto::Rect { x, y, w, h } => SelShape::Rect(Rect::from_xywh(*x, *y, *w, *h)?),
        SelDto::Poly { points } => SelShape::Poly(points.clone()),
    };
    Some(SelOp { add: d.add, shape })
}

/// Проект из заголовка и снимка: PNG кодируется здесь (можно в фоновом потоке).
pub fn build(mut header: Header, shot: &tiny_skia::Pixmap) -> Result<Vec<u8>, String> {
    let png = shot.encode_png().map_err(|e| e.to_string())?;
    header.png_len = png.len();
    encode(&header, &png)
}

pub fn encode(header: &Header, png: &[u8]) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(header).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(MAGIC.len() + 4 + json.len() + png.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(png);
    Ok(out)
}

/// Пределы для недоверенного файла.
const MAX_PIXELS: u64 = 16384 * 16384;
const MAX_SHAPES: usize = 10_000;
const MAX_POINTS: usize = 200_000;
const MAX_TEXT: usize = 10_000;

/// Размер из заголовка PNG (IHDR) без распаковки.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((w, h))
}

/// Проект приходит извне: отклоняем мусор, приводим числа к допустимым диапазонам.
fn sanitize(h: &mut Header) -> Result<(), String> {
    use crate::shapes::Kind;
    let bad = |m: &str| Err(format!("Повреждённый проект: {m}"));
    if h.width == 0 || h.height == 0 || h.width as u64 * h.height as u64 > MAX_PIXELS {
        return bad("недопустимый размер снимка");
    }
    if h.shapes.len() > MAX_SHAPES || h.selection.len() > MAX_SHAPES {
        return bad("слишком много объектов");
    }
    // Координаты в пределах снимка с запасом: фигура может выходить за край.
    let lim = (h.width.max(h.height) as f32) * 4.0;
    let ok = |v: f32| v.is_finite() && v.abs() <= lim;
    let ok_pt = |p: &(f32, f32)| ok(p.0) && ok(p.1);
    let mut points = 0usize;
    for op in &h.selection {
        match &op.shape {
            SelDto::Rect { x, y, w, h: hh } => {
                if ![*x, *y, *w, *hh].iter().all(|v| ok(*v)) || *w <= 0.0 || *hh <= 0.0 {
                    return bad("неверное выделение");
                }
            }
            SelDto::Poly { points: p } => {
                points += p.len();
                if !p.iter().all(ok_pt) {
                    return bad("неверное выделение");
                }
            }
        }
    }
    if !h.line_width.is_finite() {
        return bad("неверная толщина");
    }
    h.line_width = h.line_width.clamp(1.0, 40.0);
    h.color &= 0xFF_FFFF;
    // Фигура с неверной толщиной или координатами (например, уведённая далеко за
    // край) пропадает одна, проект открывается: иначе теряется вся разметка.
    let before = h.shapes.len();
    h.shapes.retain_mut(|s| {
        if !s.width.is_finite() {
            return false;
        }
        s.width = s.width.clamp(1.0, 40.0);
        match &s.kind {
            Kind::Pencil(p) | Kind::Marker(p) => p.iter().all(ok_pt),
            Kind::Line(a, b) | Kind::Arrow(a, b) | Kind::Rect(a, b) | Kind::Pixelate(a, b) | Kind::FilledRect(a, b) | Kind::Ellipse(a, b) => {
                ok_pt(a) && ok_pt(b)
            }
            Kind::Counter { at, n, tip } => ok_pt(at) && tip.as_ref().is_none_or(ok_pt) && *n <= 10_000,
            Kind::Ruler(a, b) => ok_pt(a) && ok_pt(b),
            // Длинный текст можно увести за край на всю его ширину: угол текста
            // проверяем с большим запасом (рисование обрезается, стоимость от длины).
            Kind::Text { at, text } => at.0.is_finite() && at.1.is_finite() && at.0.abs() <= 1.0e6 && at.1.abs() <= 1.0e6 && text.chars().count() <= MAX_TEXT,
        }
    });
    if h.shapes.len() < before {
        log::warn!("project: dropped {} shapes with invalid width or coordinates", before - h.shapes.len());
    }
    for s in &h.shapes {
        if let Kind::Pencil(p) | Kind::Marker(p) = &s.kind {
            points += p.len();
        }
    }
    if points > MAX_POINTS {
        return bad("слишком много точек");
    }
    Ok(())
}

pub fn decode(bytes: &[u8]) -> Result<(Header, &[u8]), String> {
    let bad = || "Это не проект Frostshot".to_string();
    let rest = bytes.strip_prefix(MAGIC.as_slice()).ok_or_else(bad)?;
    let len = u32::from_le_bytes(rest.get(..4).ok_or_else(bad)?.try_into().unwrap()) as usize;
    let json = rest.get(4..4 + len).ok_or_else(bad)?;
    let mut header: Header = serde_json::from_slice(json).map_err(|e| format!("Повреждённый проект: {e}"))?;
    if header.version > VERSION {
        return Err("Проект создан более новой версией Frostshot".into());
    }
    let png = &rest[4 + len..];
    if png.len() != header.png_len {
        return Err("Повреждённый проект: неполный снимок".into());
    }
    // Размер картинки проверяем до распаковки, чтобы файл не заставил выделить гигабайты.
    if png_size(png) != Some((header.width, header.height)) {
        return Err("Повреждённый проект: размер снимка не совпадает".into());
    }
    sanitize(&mut header)?;
    Ok((header, png))
}
