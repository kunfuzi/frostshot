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

pub fn encode(header: &Header, png: &[u8]) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(header).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(MAGIC.len() + 4 + json.len() + png.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(png);
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<(Header, &[u8]), String> {
    let bad = || "Это не проект Frostshot".to_string();
    let rest = bytes.strip_prefix(MAGIC.as_slice()).ok_or_else(bad)?;
    let len = u32::from_le_bytes(rest.get(..4).ok_or_else(bad)?.try_into().unwrap()) as usize;
    let json = rest.get(4..4 + len).ok_or_else(bad)?;
    let header: Header = serde_json::from_slice(json).map_err(|e| format!("Повреждённый проект: {e}"))?;
    if header.version > VERSION {
        return Err("Проект создан более новой версией Frostshot".into());
    }
    let png = &rest[4 + len..];
    if png.len() != header.png_len {
        return Err("Повреждённый проект: неполный снимок".into());
    }
    Ok((header, png))
}
