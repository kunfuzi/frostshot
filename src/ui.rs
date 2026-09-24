//! Элементы интерфейса оверлея: панели, иконки, лупа, подписи.

use crate::draw::{self, Rgb};
use crate::shapes::Tool;
use ab_glyph::FontVec;
use std::sync::atomic::{AtomicU32, Ordering};
use tiny_skia::{FillRule, LineCap, PathBuilder, Pixmap, Rect, Stroke, StrokeDash, Transform};

pub const PALETTE: [u32; 8] = [
    0xE24B4A, 0xEF9F27, 0xF5D90A, 0x3BB54A, 0x378ADD, 0x7F77DD, 0x111111, 0xFFFFFF,
];

const PANEL: Rgb = [0x1f, 0x1f, 0x22];
const FG: Rgb = [0xe6, 0xe6, 0xe6];
const MUTED: Rgb = [0x70, 0x70, 0x76];
pub const ACCENT: Rgb = [0x37, 0x8a, 0xdd];
const HOVER: Rgb = [0x3a, 0x3a, 0x40];
const WHITE: Rgb = [255, 255, 255];
/// Базовый размер шрифта интерфейса при масштабе 100% (из config.toml).
static UI_FONT_BITS: AtomicU32 = AtomicU32::new(0x4190_0000); // 18.0

pub fn set_font_size(px: f32) {
    UI_FONT_BITS.store(px.clamp(10.0, 40.0).to_bits(), Ordering::Relaxed);
}

pub fn ui_font() -> f32 {
    f32::from_bits(UI_FONT_BITS.load(Ordering::Relaxed))
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Btn {
    Tool(Tool),
    Color,
    Undo,
    Redo,
    Pin,
    Upload,
    Copy,
    Save,
    Close,
    Swatch(usize),
    SavePng,
    SaveProject,
}

impl Btn {
    pub fn tooltip(self) -> String {
        let s = match self {
            // Колесо мыши меняет толщину: говорим об этом там, где его ищут.
            Btn::Tool(t) if !t.is_selection() => return format!("{}  ·  колесо: толщина", t.label()),
            Btn::Tool(t) => t.label(),
            Btn::Color => "Цвет  ·  колесо мыши: толщина",
            Btn::Undo => "Отменить (Ctrl+Z)",
            Btn::Redo => "Повторить (Ctrl+Shift+Z)",
            Btn::Pin => "Закрепить поверх окон (P)",
            Btn::Upload => "Облако (скоро)",
            Btn::Copy => "Копировать (Ctrl+C, Enter)",
            Btn::Save => "Сохранить…",
            Btn::Close => "Закрыть (Esc)",
            Btn::Swatch(_) | Btn::SavePng | Btn::SaveProject => "",
        };
        s.to_string()
    }
}

pub struct Layout {
    pub panels: Vec<Rect>,
    pub buttons: Vec<(Btn, Rect)>,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Btn> {
        self.buttons.iter().find(|(_, r)| contains(r, x, y)).map(|(b, _)| *b)
    }
    pub fn over_panel(&self, x: f32, y: f32) -> bool {
        self.panels.iter().any(|r| contains(r, x, y))
    }
}

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left() && x < r.right() && y >= r.top() && y < r.bottom()
}

fn ov(a: Rect, b: Rect) -> bool {
    a.left() < b.right() && b.left() < a.right() && a.top() < b.bottom() && b.top() < a.bottom()
}

pub fn layout(bbox: Rect, mw: f32, mh: f32, s: f32, palette_open: bool, save_menu: bool) -> Layout {
    let b = (32.0 * s).round();
    let pad = (4.0 * s).round();
    let gap = (8.0 * s).round();
    let mut out = Layout { panels: vec![], buttons: vec![] };

    // Вертикальная панель инструментов.
    let mut vitems: Vec<Btn> = Tool::ALL.iter().map(|t| Btn::Tool(*t)).collect();
    vitems.push(Btn::Color);
    vitems.push(Btn::Undo);
    vitems.push(Btn::Redo);
    // Не помещается по высоте: переносим во второй (третий) столбец.
    let rows = (((mh - 2.0 * pad) / b).floor() as usize).clamp(1, vitems.len());
    let cols = vitems.len().div_ceil(rows);
    let (vw, vh) = (cols as f32 * b + 2.0 * pad, rows as f32 * b + 2.0 * pad);
    let mut vx = bbox.right() + gap;
    if vx + vw > mw {
        vx = bbox.left() - gap - vw;
        if vx < 0.0 {
            vx = bbox.right() - gap - vw;
        }
    }
    let vy = bbox.top().min(mh - vh).max(0.0);
    let vpanel = Rect::from_xywh(vx, vy, vw, vh).unwrap();
    out.panels.push(vpanel);
    for (i, it) in vitems.iter().enumerate() {
        let (c, r) = ((i / rows) as f32, (i % rows) as f32);
        out.buttons.push((*it, Rect::from_xywh(vx + pad + c * b, vy + pad + r * b, b, b).unwrap()));
    }

    // Горизонтальная панель действий.
    let hitems = [Btn::Upload, Btn::Pin, Btn::Copy, Btn::Save, Btn::Close];
    let (hw, hh) = (hitems.len() as f32 * b + 2.0 * pad, b + 2.0 * pad);
    let mut hy = bbox.bottom() + gap;
    if hy + hh > mh {
        hy = bbox.top() - gap - hh;
        if hy < 0.0 {
            hy = bbox.bottom() - gap - hh;
        }
    }
    let mut hx = (bbox.right() - hw).min(mw - hw).max(0.0);
    let mut hpanel = Rect::from_xywh(hx, hy, hw, hh).unwrap();
    if ov(hpanel, vpanel) {
        hx = if vx - gap - hw >= 0.0 { vx - gap - hw } else { vx + vw + gap };
        hpanel = Rect::from_xywh(hx, hy, hw, hh).unwrap();
    }
    out.panels.push(hpanel);
    for (i, it) in hitems.iter().enumerate() {
        out.buttons.push((*it, Rect::from_xywh(hx + pad + i as f32 * b, hy + pad, b, b).unwrap()));
    }

    // Меню сохранения над (или под) панелью действий.
    if save_menu {
        let mh_item = b;
        let items = [Btn::SavePng, Btn::SaveProject];
        let mw_ = (300.0 * s * ui_font() / 18.0).round();
        let mhh = items.len() as f32 * mh_item + 2.0 * pad;
        let save_r = out.buttons.iter().find(|(b, _)| *b == Btn::Save).unwrap().1;
        let mx = (save_r.right() - mw_).clamp(0.0, (mw - mw_).max(0.0));
        let my = if hpanel.top() - gap - mhh >= 0.0 { hpanel.top() - gap - mhh } else { hpanel.bottom() + gap };
        out.panels.push(Rect::from_xywh(mx, my, mw_, mhh).unwrap());
        for (i, it) in items.iter().enumerate() {
            out.buttons.push((*it, Rect::from_xywh(mx + pad, my + pad + i as f32 * mh_item, mw_ - 2.0 * pad, mh_item).unwrap()));
        }
    }

    // Палитра рядом с кнопкой цвета.
    if palette_open {
        let cy = out.buttons.iter().find(|(b, _)| *b == Btn::Color).unwrap().1.top() - pad;
        let pw = PALETTE.len() as f32 * b + 2.0 * pad;
        let px = if vx - gap - pw >= 0.0 { vx - gap - pw } else { vx + vw + gap };
        out.panels.push(Rect::from_xywh(px, cy, pw, b + 2.0 * pad).unwrap());
        for i in 0..PALETTE.len() {
            out.buttons.push((Btn::Swatch(i), Rect::from_xywh(px + pad + i as f32 * b, cy + pad, b, b).unwrap()));
        }
    }
    out
}

pub struct UiState<'a> {
    pub tool: Tool,
    pub color: u32,
    pub hover: Option<Btn>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub font: Option<&'a FontVec>,
    pub scale: f32,
}

pub fn draw_layout(pm: &mut Pixmap, l: &Layout, st: &UiState) {
    let s = st.scale;
    for p in &l.panels {
        draw::fill_rounded(pm, *p, 6.0 * s, PANEL, 0.94);
    }
    for (btn, r) in &l.buttons {
        let active = matches!(btn, Btn::Tool(t) if *t == st.tool);
        let r2 = r.inset(2.0 * s, 2.0 * s).unwrap_or(*r);
        if active {
            draw::fill_rounded(pm, r2, 4.0 * s, ACCENT, 1.0);
        } else if st.hover == Some(*btn) {
            draw::fill_rounded(pm, r2, 4.0 * s, HOVER, 1.0);
        }
        let fg = match btn {
            Btn::Upload => MUTED,
            Btn::Undo if !st.can_undo => MUTED,
            Btn::Redo if !st.can_redo => MUTED,
            _ if active => WHITE,
            _ => FG,
        };
        match (btn, st.font) {
            (Btn::SavePng | Btn::SaveProject, Some(font)) => {
                let (label, hint) = if *btn == Btn::SavePng { ("PNG…", "Ctrl+S") } else { ("Проект .frost…", "для доработки") };
                let size = ui_font() * s;
                let ty = r.top() + (r.height() - draw::line_height(font, size)) / 2.0;
                draw::draw_text(pm, font, label, r.left() + 10.0 * s, ty, size, FG, 1.0, None);
                let hs = (ui_font() - 3.0) * s;
                let hw = draw::text_size(font, hint, hs).0;
                let hy = r.top() + (r.height() - draw::line_height(font, hs)) / 2.0;
                draw::draw_text(pm, font, hint, r.right() - hw - 10.0 * s, hy, hs, MUTED, 1.0, None);
            }
            _ => icon(pm, *btn, *r, fg, st),
        }
    }
    if let (Some(h), Some(font)) = (st.hover, st.font) {
        let tip = h.tooltip();
        if !tip.is_empty() {
            if let Some((_, r)) = l.buttons.iter().find(|(b, _)| *b == h) {
                tooltip(pm, font, &tip, *r, s);
            }
        }
    }
}

fn tooltip(pm: &mut Pixmap, font: &FontVec, text: &str, anchor: Rect, s: f32) {
    let size = ui_font() * s;
    let (tw, th) = draw::text_size(font, text, size);
    let pad = 6.0 * s;
    let (w, h) = (tw + 2.0 * pad, th + pad);
    let (mw, mh) = (pm.width() as f32, pm.height() as f32);
    let mut x = anchor.left() - w - 6.0 * s;
    if x < 0.0 {
        x = anchor.right() + 6.0 * s;
    }
    let mut y = anchor.top() + (anchor.height() - h) / 2.0;
    if x + w > mw {
        x = (anchor.left() + anchor.width() / 2.0 - w / 2.0).clamp(0.0, (mw - w).max(0.0));
        y = anchor.top() - h - 6.0 * s;
    }
    y = y.clamp(0.0, (mh - h).max(0.0));
    draw::fill_rounded(pm, Rect::from_xywh(x, y, w, h).unwrap(), 4.0 * s, [0, 0, 0], 0.85);
    draw::draw_text(pm, font, text, x + pad, y + pad / 2.0, size, WHITE, 1.0, None);
}

fn icon(pm: &mut Pixmap, btn: Btn, r: Rect, fg: Rgb, st: &UiState) {
    let k = r.width() / 32.0;
    let (ox, oy) = (r.left(), r.top());
    let p = |x: f32, y: f32| (ox + x * k, oy + y * k);
    let w = 2.0 * k;
    let ln = |pm: &mut Pixmap, a: (f32, f32), b: (f32, f32), wd: f32, alpha: f32| {
        let (a, b) = (p(a.0, a.1), p(b.0, b.1));
        draw::line(pm, a.0, a.1, b.0, b.1, fg, alpha, wd, None);
    };
    let rect_outline = |pm: &mut Pixmap, x: f32, y: f32, rw: f32, rh: f32, dashed: bool| {
        let (a, _) = (p(x, y), 0);
        if let Some(rr) = Rect::from_xywh(a.0, a.1, rw * k, rh * k) {
            let path = PathBuilder::from_rect(rr);
            let mut stroke = Stroke { width: w * 0.8, ..Stroke::default() };
            if dashed {
                stroke.dash = StrokeDash::new(vec![3.0 * k, 2.0 * k], 0.0);
            }
            pm.stroke_path(&path, &draw::paint(fg, 1.0), &stroke, Transform::identity(), None);
        }
    };
    let tri = |pm: &mut Pixmap, a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        let (a, b, c) = (p(a.0, a.1), p(b.0, b.1), p(c.0, c.1));
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        pb.line_to(c.0, c.1);
        pb.close();
        if let Some(path) = pb.finish() {
            pm.fill_path(&path, &draw::paint(fg, 1.0), FillRule::Winding, Transform::identity(), None);
        }
    };
    match btn {
        Btn::Tool(Tool::SelectRect) => rect_outline(pm, 9.0, 9.0, 14.0, 14.0, true),
        Btn::Tool(Tool::SelectLasso) => {
            let c = p(16.0, 14.0);
            if let Some(path) = PathBuilder::from_circle(c.0, c.1, 7.0 * k) {
                let stroke = Stroke {
                    width: w * 0.8,
                    dash: StrokeDash::new(vec![3.0 * k, 2.0 * k], 0.0),
                    ..Stroke::default()
                };
                pm.stroke_path(&path, &draw::paint(fg, 1.0), &stroke, Transform::identity(), None);
            }
            ln(pm, (12.0, 20.0), (10.0, 25.0), w, 1.0);
        }
        Btn::Tool(Tool::Pencil) => {
            ln(pm, (10.0, 22.0), (21.0, 11.0), w * 1.4, 1.0);
            ln(pm, (8.5, 23.5), (10.0, 22.0), w * 0.8, 1.0);
        }
        Btn::Tool(Tool::Marker) => {
            ln(pm, (10.0, 19.0), (21.0, 12.0), w * 3.0, 0.6);
            ln(pm, (8.0, 24.0), (24.0, 24.0), w, 1.0);
        }
        Btn::Tool(Tool::Line) => ln(pm, (9.0, 23.0), (23.0, 9.0), w, 1.0),
        Btn::Tool(Tool::Arrow) => {
            ln(pm, (9.0, 23.0), (19.0, 13.0), w, 1.0);
            tri(pm, (23.5, 8.5), (15.0, 12.0), (20.0, 17.0));
        }
        Btn::Tool(Tool::Rect) => rect_outline(pm, 8.0, 10.0, 16.0, 12.0, false),
        Btn::Tool(Tool::FilledRect) => {
            let a = p(8.0, 10.0);
            if let Some(rr) = Rect::from_xywh(a.0, a.1, 16.0 * k, 12.0 * k) {
                draw::fill_rect(pm, rr, fg, 1.0, None);
            }
        }
        Btn::Tool(Tool::Ellipse) => {
            let a = p(7.0, 10.0);
            if let Some(path) = Rect::from_xywh(a.0, a.1, 18.0 * k, 12.0 * k).and_then(PathBuilder::from_oval) {
                draw::stroke_path(pm, &path, fg, 1.0, w * 0.8, None);
            }
        }
        Btn::Tool(Tool::Counter) => {
            let c = p(16.0, 16.0);
            draw::circle(pm, c.0, c.1, 8.5 * k, fg, 1.0, false, w * 0.8);
            if let Some(font) = st.font {
                let size = 12.0 * k;
                let (tw, th) = draw::text_size(font, "1", size);
                draw::draw_text(pm, font, "1", c.0 - tw / 2.0, c.1 - th / 2.0, size, fg, 1.0, None);
            }
        }
        Btn::Tool(Tool::Text) => {
            ln(pm, (10.0, 10.0), (22.0, 10.0), w * 1.2, 1.0);
            ln(pm, (16.0, 10.0), (16.0, 23.0), w * 1.2, 1.0);
        }
        Btn::Tool(Tool::Pixelate) => {
            for i in 0..3 {
                for j in 0..3 {
                    let a = p(9.0 + i as f32 * 5.0, 9.0 + j as f32 * 5.0);
                    let alpha = if (i + j) % 2 == 0 { 1.0 } else { 0.4 };
                    if let Some(rr) = Rect::from_xywh(a.0, a.1, 4.5 * k, 4.5 * k) {
                        draw::fill_rect(pm, rr, fg, alpha, None);
                    }
                }
            }
        }
        Btn::Color => {
            let c = p(16.0, 16.0);
            draw::circle(pm, c.0, c.1, 8.0 * k, draw::rgb(st.color), 1.0, true, 0.0);
            draw::circle(pm, c.0, c.1, 8.0 * k, WHITE, 1.0, false, 1.5 * k);
        }
        Btn::Swatch(i) => {
            let c = p(16.0, 16.0);
            draw::circle(pm, c.0, c.1, 9.0 * k, draw::rgb(PALETTE[i]), 1.0, true, 0.0);
            let ring = if PALETTE[i] == st.color { ACCENT } else { MUTED };
            draw::circle(pm, c.0, c.1, 9.0 * k, ring, 1.0, false, if PALETTE[i] == st.color { 3.0 * k } else { 1.0 * k });
        }
        Btn::Undo => {
            let mut pb = PathBuilder::new();
            let (a, b1, c1, d, e, f) = (p(11.0, 14.0), p(23.0, 14.0), p(23.0, 18.0), p(23.0, 22.0), p(19.0, 22.0), p(13.0, 22.0));
            pb.move_to(a.0, a.1);
            pb.line_to(b1.0 - 4.0 * k, b1.1);
            pb.quad_to(b1.0, b1.1, c1.0, c1.1);
            pb.quad_to(d.0, d.1, e.0, e.1);
            pb.line_to(f.0, f.1);
            if let Some(path) = pb.finish() {
                draw::stroke_path(pm, &path, fg, 1.0, w, None);
            }
            tri(pm, (8.0, 14.0), (14.0, 9.0), (14.0, 19.0));
        }
        Btn::Redo => {
            // Зеркало иконки «Отменить».
            let q = |x: f32, y: f32| p(32.0 - x, y);
            let mut pb = PathBuilder::new();
            let (a, b1, c1, d, e, f) = (q(11.0, 14.0), q(19.0, 14.0), q(23.0, 18.0), q(23.0, 22.0), q(19.0, 22.0), q(13.0, 22.0));
            let (bx, _) = q(23.0, 14.0);
            pb.move_to(a.0, a.1);
            pb.line_to(b1.0, b1.1);
            pb.quad_to(bx, b1.1, c1.0, c1.1);
            pb.quad_to(d.0, d.1, e.0, e.1);
            pb.line_to(f.0, f.1);
            if let Some(path) = pb.finish() {
                draw::stroke_path(pm, &path, fg, 1.0, w, None);
            }
            tri(pm, (24.0, 14.0), (18.0, 9.0), (18.0, 19.0));
        }
        Btn::Pin => {
            // Канцелярская кнопка: шляпка, корпус, игла.
            ln(pm, (12.0, 9.0), (20.0, 9.0), w * 1.2, 1.0);
            let a = p(13.5, 9.0);
            if let Some(rr) = Rect::from_xywh(a.0, a.1, 5.0 * k, 8.0 * k) {
                draw::fill_rect(pm, rr, fg, 1.0, None);
            }
            ln(pm, (10.0, 18.0), (22.0, 18.0), w * 1.2, 1.0);
            ln(pm, (16.0, 18.0), (16.0, 25.0), w * 0.8, 1.0);
        }
        Btn::Upload => {
            ln(pm, (16.0, 22.0), (16.0, 12.0), w, 1.0);
            tri(pm, (16.0, 8.0), (11.0, 14.0), (21.0, 14.0));
            ln(pm, (9.0, 24.0), (23.0, 24.0), w, 1.0);
        }
        Btn::Copy => {
            rect_outline(pm, 12.0, 8.0, 11.0, 13.0, false);
            let a = p(9.0, 11.0);
            if let Some(rr) = Rect::from_xywh(a.0, a.1, 11.0 * k, 13.0 * k) {
                draw::fill_rect(pm, rr, PANEL, 1.0, None);
            }
            rect_outline(pm, 9.0, 11.0, 11.0, 13.0, false);
        }
        Btn::Save => {
            rect_outline(pm, 9.0, 9.0, 14.0, 14.0, false);
            rect_outline(pm, 12.0, 9.0, 8.0, 5.0, false);
            rect_outline(pm, 12.0, 17.0, 8.0, 6.0, false);
        }
        Btn::Close => {
            ln(pm, (10.0, 10.0), (22.0, 22.0), w, 1.0);
            ln(pm, (22.0, 10.0), (10.0, 22.0), w, 1.0);
        }
        Btn::SavePng | Btn::SaveProject => {}
    }
}

/// Лупа у курсора: 15x15 пикселей снимка, координаты и цвет.
#[allow(clippy::too_many_arguments)]
pub fn magnifier(pm: &mut Pixmap, src: &Pixmap, cx: f32, cy: f32, sx: f32, sy: f32, s: f32, font: Option<&FontVec>) {
    const N: i32 = 15;
    let cell = (8.0 * s).round().max(4.0);
    let size = cell * N as f32;
    let info_h = if font.is_some() { (ui_font() + 10.0) * s } else { 0.0 };
    let off = 20.0 * s;
    let (mw, mh) = (pm.width() as f32, pm.height() as f32);
    let mut x = cx + off;
    let mut y = cy + off;
    if x + size > mw {
        x = cx - off - size;
    }
    if y + size + info_h > mh {
        y = cy - off - size - info_h;
    }
    let (px, py) = (sx.floor() as i32, sy.floor() as i32);
    let d = src.data();
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    let mut center = [0u8; 3];
    for j in 0..N {
        for i in 0..N {
            let (sx, sy) = (px - N / 2 + i, py - N / 2 + j);
            let c = if sx >= 0 && sy >= 0 && sx < sw && sy < sh {
                let k = ((sy * sw + sx) * 4) as usize;
                [d[k], d[k + 1], d[k + 2]]
            } else {
                [0, 0, 0]
            };
            if i == N / 2 && j == N / 2 {
                center = c;
            }
            if let Some(r) = Rect::from_xywh(x + i as f32 * cell, y + j as f32 * cell, cell, cell) {
                draw::fill_rect(pm, r, c, 1.0, None);
            }
        }
    }
    // Перекрестье и рамка.
    let mid = x + (N / 2) as f32 * cell;
    let midy = y + (N / 2) as f32 * cell;
    if let Some(r) = Rect::from_xywh(mid, midy, cell, cell) {
        let path = PathBuilder::from_rect(r);
        let stroke = Stroke { width: 1.0, line_cap: LineCap::Square, ..Stroke::default() };
        pm.stroke_path(&path, &draw::paint(ACCENT, 1.0), &stroke, Transform::identity(), None);
    }
    if let Some(r) = Rect::from_xywh(x, y, size, size) {
        let path = PathBuilder::from_rect(r);
        let stroke = Stroke { width: 2.0 * s, ..Stroke::default() };
        pm.stroke_path(&path, &draw::paint(WHITE, 1.0), &stroke, Transform::identity(), None);
    }
    if let Some(font) = font {
        let text = format!("{px}, {py}   #{:02X}{:02X}{:02X}", center[0], center[1], center[2]);
        if let Some(r) = Rect::from_xywh(x, y + size, size, info_h) {
            draw::fill_rect(pm, r, [0, 0, 0], 0.8, None);
        }
        draw::draw_text(pm, font, &text, x + 6.0 * s, y + size + 3.0 * s, (ui_font() - 1.0) * s, WHITE, 1.0, None);
    }
}

/// Плашка с текстом (размер выделения, подсказки).
pub fn label(pm: &mut Pixmap, font: &FontVec, text: &str, x: f32, y: f32, s: f32) -> Rect {
    let size = ui_font() * s;
    let (tw, th) = draw::text_size(font, text, size);
    let pad = 6.0 * s;
    let r = Rect::from_xywh(x, y, tw + 2.0 * pad, th + pad).unwrap();
    draw::fill_rounded(pm, r, 4.0 * s, [0, 0, 0], 0.75);
    draw::draw_text(pm, font, text, x + pad, y + pad / 2.0, size, WHITE, 1.0, None);
    r
}

pub fn label_size(font: &FontVec, text: &str, s: f32) -> (f32, f32) {
    let (tw, th) = draw::text_size(font, text, ui_font() * s);
    (tw + 12.0 * s, th + 6.0 * s)
}

/// Плашка у курсора после прокрутки колеса: образец линии текущего цвета и толщины
/// (как она выглядит на экране при текущем масштабе) и подпись со значением.
#[allow(clippy::too_many_arguments)]
pub fn width_hint(pm: &mut Pixmap, font: &FontVec, tool: Tool, width: f32, c: Rgb, zoom: f32, cx: f32, cy: f32, s: f32) {
    let size = ui_font() * s;
    let (label, sample_h) = match tool {
        Tool::Text => (format!("Размер текста {}", crate::shapes::font_size(width).round()), crate::shapes::font_size(width) * zoom),
        Tool::Counter => (format!("Размер номера {}", width.round()), crate::shapes::counter_radius(width) * 2.0 * zoom),
        Tool::Marker => (format!("Толщина {}", width.round()), crate::shapes::marker_width(width) * zoom),
        Tool::Pixelate => (format!("Блок {} px", (8.0 + width * 2.0).round()), 24.0 * s),
        _ => (format!("Толщина {}", width.round()), width * zoom),
    };
    let sample_h = sample_h.clamp(2.0, 120.0 * s);
    let sample_w = (96.0 * s).max(sample_h * 1.6);
    let pad = 10.0 * s;
    let (tw, th) = draw::text_size(font, &label, size);
    let (hs, hint) = ((ui_font() - 3.0) * s, "колесо мыши");
    let (hw, hh) = draw::text_size(font, hint, hs);
    let w = sample_w.max(tw).max(hw) + 2.0 * pad;
    let h = pad + sample_h + pad * 0.6 + th + hh + pad;
    let (mw, mh) = (pm.width() as f32, pm.height() as f32);
    // Справа вверху от курсора: справа внизу стоит лупа.
    let mut x = cx + 24.0 * s;
    let mut y = cy - 24.0 * s - h;
    if x + w > mw {
        x = cx - 24.0 * s - w;
    }
    if y < 0.0 {
        y = cy + 24.0 * s;
        if y + h > mh {
            y = mh - h;
        }
    }
    let (x, y) = (x.max(0.0), y.max(0.0));
    if let Some(r) = Rect::from_xywh(x, y, w, h) {
        draw::fill_rounded(pm, r, 6.0 * s, PANEL, 0.94);
    }
    let (sx, sy) = (x + (w - sample_w) / 2.0, y + pad + sample_h / 2.0);
    match tool {
        Tool::Text => {
            let fs = sample_h.max(8.0);
            let aw = draw::text_size(font, "Aa", fs).0;
            draw::draw_text(pm, font, "Aa", x + (w - aw) / 2.0, sy - draw::ascent(font, fs) * 0.8, fs, c, 1.0, None);
        }
        Tool::Counter => draw::circle(pm, x + w / 2.0, sy, sample_h / 2.0, c, 1.0, true, 0.0),
        Tool::Pixelate => {
            let b = sample_h / 2.0;
            for i in 0..((sample_w / b) as i32) {
                for j in 0..2 {
                    let a = if (i + j) % 2 == 0 { 0.9 } else { 0.45 };
                    if let Some(r) = Rect::from_xywh(sx + i as f32 * b, y + pad + j as f32 * b, b, b) {
                        draw::fill_rect(pm, r, FG, a, None);
                    }
                }
            }
        }
        Tool::Marker => draw::line(pm, sx + sample_h / 2.0, sy, sx + sample_w - sample_h / 2.0, sy, c, 0.4, sample_h, None),
        _ => draw::line(pm, sx + sample_h / 2.0, sy, sx + sample_w - sample_h / 2.0, sy, c, 1.0, sample_h, None),
    }
    let ty = y + pad + sample_h + pad * 0.6;
    draw::draw_text(pm, font, &label, x + (w - tw) / 2.0, ty, size, WHITE, 1.0, None);
    draw::draw_text(pm, font, hint, x + (w - hw) / 2.0, ty + th, hs, MUTED, 1.0, None);
}
