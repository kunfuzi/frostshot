//! Фигуры разметки. Координаты в пикселях снимка монитора (инвариант 8):
//! одна функция рендера и для экрана, и для итогового файла.

use crate::draw::{self, Rgb};
use ab_glyph::FontVec;
use serde::{Deserialize, Serialize};
use tiny_skia::{FillRule, Mask, PathBuilder, Pixmap, Transform};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    /// Курсор (V): выбрать, двигать и менять готовые фигуры.
    Pointer,
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
    pub const ALL: [Tool; 14] = [
        Tool::Pointer,
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

    /// Инструмент, которым рисуют такую фигуру (подпись образца толщины).
    pub fn of(kind: &Kind) -> Tool {
        match kind {
            Kind::Pencil(_) => Tool::Pencil,
            Kind::Marker(_) => Tool::Marker,
            Kind::Line(..) => Tool::Line,
            Kind::Arrow(..) => Tool::Arrow,
            Kind::Rect(..) => Tool::Rect,
            Kind::Text { .. } => Tool::Text,
            Kind::Pixelate(..) => Tool::Pixelate,
            Kind::FilledRect(..) => Tool::FilledRect,
            Kind::Ellipse(..) => Tool::Ellipse,
            Kind::Counter { .. } => Tool::Counter,
            Kind::Ruler(..) => Tool::Ruler,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Pointer => "Курсор (V): выбрать и двигать фигуры",
            Tool::SelectRect => "Рамка (M) · Shift: добавить, Alt: вычесть",
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

/// Расстояние от точки до отрезка.
pub fn seg_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (a.0 + dx * t - p.0, a.1 + dy * t - p.1);
    (qx * qx + qy * qy).sqrt()
}

fn norm(a: Pt, b: Pt) -> (f32, f32, f32, f32) {
    (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
}

/// Правка готовых фигур в режиме выделения: попадание, габарит, ручки, сдвиг.
impl Shape {
    /// Габарит (l, t, r, b) в координатах снимка, с учётом толщины линии.
    /// mm: миллиметров на пиксель (подпись линейки влияет на её размер).
    pub fn bounds(&self, font: Option<&FontVec>, mm: Option<f32>) -> (f32, f32, f32, f32) {
        let pad = match &self.kind {
            Kind::Marker(_) => marker_width(self.width) / 2.0,
            // Наконечник стрелки шире линии.
            Kind::Arrow(..) => (self.width / 2.0).max(arrow_head(self.width) / 2.0),
            // Засечки линейки поперёк линии, с круглыми концами штриха.
            Kind::Ruler(..) => ruler_tick(self.width) + ruler_lw(self.width) / 2.0,
            _ => self.width / 2.0,
        };
        let grow = |(l, t, r, b): (f32, f32, f32, f32), d: f32| (l - d, t - d, r + d, b + d);
        match &self.kind {
            Kind::Pencil(p) | Kind::Marker(p) => {
                let mut bb = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for q in p {
                    bb = (bb.0.min(q.0), bb.1.min(q.1), bb.2.max(q.0), bb.3.max(q.1));
                }
                grow(bb, pad)
            }
            Kind::Ruler(a, b) => {
                let bb = grow(norm(*a, *b), pad);
                match font.and_then(|f| ruler_plate(*a, *b, self.width, f, mm)) {
                    Some(pl) => (bb.0.min(pl.x), bb.1.min(pl.y), bb.2.max(pl.x + pl.w), bb.3.max(pl.y + pl.h)),
                    None => bb,
                }
            }
            Kind::Line(a, b) | Kind::Arrow(a, b) | Kind::Rect(a, b) | Kind::Ellipse(a, b) => grow(norm(*a, *b), pad),
            Kind::FilledRect(a, b) | Kind::Pixelate(a, b) => norm(*a, *b),
            Kind::Text { at, text } => {
                let size = font_size(self.width);
                let (w, h) = font.map_or((text.chars().count() as f32 * size * 0.55, size * 1.3), |f| draw::text_size(f, text, size));
                (at.0, at.1, at.0 + w, at.1 + h)
            }
            Kind::Counter { at, tip, .. } => {
                let r = counter_radius(self.width);
                let (mut l, mut t, mut rr, mut b) = (at.0 - r, at.1 - r, at.0 + r, at.1 + r);
                if let Some(q) = tip {
                    (l, t, rr, b) = (l.min(q.0), t.min(q.1), rr.max(q.0), b.max(q.1));
                }
                (l, t, rr, b)
            }
        }
    }

    /// Габарит самой геометрии для ограничения сдвига: у линии, стрелки и линейки
    /// без запаса на толщину и наконечник (он раздувает габарит вдоль оси).
    pub fn extent(&self, font: Option<&FontVec>, mm: Option<f32>) -> (f32, f32, f32, f32) {
        match &self.kind {
            Kind::Line(a, b) | Kind::Arrow(a, b) | Kind::Ruler(a, b) => norm(*a, *b),
            _ => self.bounds(font, mm),
        }
    }

    /// Попадает ли точка p в фигуру; tol: запас в пикселях снимка.
    /// Контурные фигуры ловятся по линии, закрашенные по площади.
    pub fn hit(&self, p: Pt, tol: f32, font: Option<&FontVec>, mm: Option<f32>) -> bool {
        let near_poly = |pts: &[Pt], d: f32| match pts {
            [] => false,
            [a] => seg_dist(p, *a, *a) <= d,
            _ => pts.windows(2).any(|w| seg_dist(p, w[0], w[1]) <= d),
        };
        let inside = |(l, t, r, b): (f32, f32, f32, f32)| p.0 >= l - tol && p.0 <= r + tol && p.1 >= t - tol && p.1 <= b + tol;
        let d = self.width / 2.0 + tol;
        match &self.kind {
            Kind::Pencil(pts) => near_poly(pts, d),
            Kind::Marker(pts) => near_poly(pts, marker_width(self.width) / 2.0 + tol),
            Kind::Line(a, b) => seg_dist(p, *a, *b) <= d,
            // Наконечник: весь нарисованный треугольник (с запасом tol).
            Kind::Arrow(a, b) => seg_dist(p, *a, *b) <= d || arrow_head_tri(*a, *b, self.width).is_some_and(|tri| near_tri(p, tri, tol)),
            // Линейка: линия, засечки на концах (толщиной в штрих), плашка с подписью.
            Kind::Ruler(a, b) => {
                let lw = ruler_lw(self.width) / 2.0 + tol;
                let len = seg_dist(*a, *b, *b);
                let ticks = len >= 1.0 && {
                    let t = ruler_tick(self.width);
                    let (px, py) = (-(b.1 - a.1) / len * t, (b.0 - a.0) / len * t);
                    [*a, *b].iter().any(|e| seg_dist(p, (e.0 - px, e.1 - py), (e.0 + px, e.1 + py)) <= lw)
                };
                seg_dist(p, *a, *b) <= lw.max(d) || ticks || {
                    font.and_then(|f| ruler_plate(*a, *b, self.width, f, mm))
                        .is_some_and(|pl| inside((pl.x, pl.y, pl.x + pl.w, pl.y + pl.h)))
                }
            }
            Kind::Rect(a, b) => {
                let (l, t, r, bb) = norm(*a, *b);
                near_poly(&[(l, t), (r, t), (r, bb), (l, bb), (l, t)], d)
            }
            Kind::Ellipse(a, b) => {
                let (l, t, r, bb) = norm(*a, *b);
                // Расстояние до контура по ломаной из 72 точек: верно и для вытянутого,
                // плоского и маленького эллипса (формулы-приближения врут на краях).
                let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + bb) / 2.0, (r - l) / 2.0, (bb - t) / 2.0);
                let pts: Vec<Pt> = (0..=72)
                    .map(|i| {
                        let a = i as f32 / 72.0 * std::f32::consts::TAU;
                        (cx + rx * a.cos(), cy + ry * a.sin())
                    })
                    .collect();
                near_poly(&pts, d)
            }
            Kind::FilledRect(a, b) | Kind::Pixelate(a, b) => inside(norm(*a, *b)),
            Kind::Text { .. } => inside(self.bounds(font, None)),
            Kind::Counter { at, tip, .. } => {
                let r = counter_radius(self.width);
                seg_dist(p, *at, *at) <= r + tol || tip.is_some_and(|q| seg_dist(p, *at, q) <= r * 0.3 + tol)
            }
        }
    }

    /// Ручки для изменения: концы линии, углы рамки, кружок и остриё счётчика.
    pub fn handles(&self) -> Vec<Pt> {
        match &self.kind {
            Kind::Line(a, b) | Kind::Arrow(a, b) | Kind::Ruler(a, b) => vec![*a, *b],
            Kind::Rect(a, b) | Kind::FilledRect(a, b) | Kind::Ellipse(a, b) | Kind::Pixelate(a, b) => {
                let (l, t, r, bb) = norm(*a, *b);
                vec![(l, t), (r, t), (r, bb), (l, bb)]
            }
            Kind::Counter { at, tip: Some(q), .. } => vec![*at, *q],
            _ => Vec::new(),
        }
    }

    /// Передвинуть ручку i в точку p (номера как в handles()).
    pub fn set_handle(&mut self, i: usize, p: Pt) {
        match &mut self.kind {
            Kind::Line(a, b) | Kind::Arrow(a, b) | Kind::Ruler(a, b) => *(if i == 0 { a } else { b }) = p,
            Kind::Rect(a, b) | Kind::FilledRect(a, b) | Kind::Ellipse(a, b) | Kind::Pixelate(a, b) => {
                let (mut l, mut t, mut r, mut bb) = norm(*a, *b);
                match i {
                    0 => (l, t) = p,
                    1 => (r, t) = p,
                    2 => (r, bb) = p,
                    _ => (l, bb) = p,
                }
                (*a, *b) = ((l, t), (r, bb));
            }
            Kind::Counter { at, tip: Some(q), .. } => *(if i == 0 { at } else { q }) = p,
            _ => {}
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        let mv = |q: &mut Pt| *q = (q.0 + dx, q.1 + dy);
        match &mut self.kind {
            Kind::Pencil(p) | Kind::Marker(p) => p.iter_mut().for_each(mv),
            Kind::Line(a, b)
            | Kind::Arrow(a, b)
            | Kind::Rect(a, b)
            | Kind::Pixelate(a, b)
            | Kind::FilledRect(a, b)
            | Kind::Ellipse(a, b)
            | Kind::Ruler(a, b) => {
                mv(a);
                mv(b);
            }
            Kind::Text { at, .. } => mv(at),
            Kind::Counter { at, tip, .. } => {
                mv(at);
                if let Some(q) = tip {
                    mv(q);
                }
            }
        }
    }

    /// Толщину меняют и у пикселизации (размер блока), но не у закрашенного прямоугольника.
    pub fn has_width(&self) -> bool {
        !matches!(self.kind, Kind::FilledRect(..))
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

/// mm: миллиметров на пиксель снимка (подпись линейки), None: только пиксели.
pub fn render(pm: &mut Pixmap, s: &Shape, src: &Pixmap, font: Option<&FontVec>, clip: Option<&Mask>, mm: Option<f32>) {
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
        Kind::Ruler(a, b) => ruler(pm, *a, *b, c, s.width, font, clip, mm),
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

/// Подпись длины: «240 px · 63,5 мм», для наклонной линии ещё проекции «(200 × 133)».
/// mm: миллиметров на пиксель монитора, None: только пиксели.
pub fn ruler_label(a: Pt, b: Pt, mm: Option<f32>) -> String {
    let (dx, dy) = ((b.0 - a.0).abs().round(), (b.1 - a.1).abs().round());
    let len = (dx * dx + dy * dy).sqrt().round();
    let mut s = if dx > 0.0 && dy > 0.0 { format!("{len} px ({dx} × {dy})") } else { format!("{len} px") };
    if let Some(k) = mm {
        s += &format!(" · {} мм", format!("{:.1}", len * k).replace('.', ","));
    }
    s
}

/// Кегль подписи линейки.
pub fn ruler_font(w: f32) -> f32 {
    (16.0 + w * 1.5).max(18.0)
}

/// Длина наконечника стрелки (он же его ширина).
pub fn arrow_head(w: f32) -> f32 {
    (w * 4.0).max(14.0)
}

/// Засечка линейки: сколько торчит поперёк линии в каждую сторону.
pub fn ruler_tick(w: f32) -> f32 {
    6.0 + w
}

/// Толщина штриха линейки и засечек.
pub fn ruler_lw(w: f32) -> f32 {
    (w * 0.5).max(1.5)
}

/// Треугольник наконечника стрелки a -> b: остриё и два угла основания.
pub fn arrow_head_tri(a: Pt, b: Pt, w: f32) -> Option<[Pt; 3]> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = arrow_head(w).min(len);
    let half = head * 0.5;
    let base = (b.0 - ux * head, b.1 - uy * head);
    Some([b, (base.0 - uy * half, base.1 + ux * half), (base.0 + uy * half, base.1 - ux * half)])
}

/// Точка внутри треугольника или не дальше tol от его сторон.
fn near_tri(p: Pt, [a, b, c]: [Pt; 3], tol: f32) -> bool {
    let cross = |o: Pt, u: Pt, v: Pt| (u.0 - o.0) * (v.1 - o.1) - (u.1 - o.1) * (v.0 - o.0);
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let inside = !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0));
    inside || seg_dist(p, a, b) <= tol || seg_dist(p, b, c) <= tol || seg_dist(p, c, a) <= tol
}

/// Плашка подписи линейки: текст, кегль, прямоугольник (x, y, w, h), отступ текста.
pub struct Plate {
    pub text: String,
    pub size: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub pad: f32,
}

/// Где стоит плашка подписи линейки: сбоку от середины, чтобы не закрывать линию.
pub fn ruler_plate(a: Pt, b: Pt, w: f32, font: &FontVec, mm: Option<f32>) -> Option<Plate> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return None;
    }
    let (px, py) = (-dy / len, dx / len);
    let text = ruler_label(a, b, mm);
    let size = ruler_font(w);
    let (tw, th) = draw::text_size(font, &text, size);
    let pad = 6.0;
    let off = ruler_tick(w) + th / 2.0 + pad;
    let (mx, my) = ((a.0 + b.0) / 2.0 + px * off, (a.1 + b.1) / 2.0 + py * off);
    Some(Plate { text, size, x: mx - tw / 2.0 - pad, y: my - th / 2.0 - pad / 2.0, w: tw + 2.0 * pad, h: th + pad, pad })
}

/// Линейка: тонкая линия, засечки на концах, подпись длины на плашке у середины.
#[allow(clippy::too_many_arguments)]
fn ruler(pm: &mut Pixmap, a: Pt, b: Pt, c: Rgb, w: f32, font: Option<&FontVec>, clip: Option<&Mask>, mm: Option<f32>) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let lw = ruler_lw(w);
    let (px, py) = (-dy / len, dx / len);
    let t = ruler_tick(w);
    draw::line(pm, a.0, a.1, b.0, b.1, c, 1.0, lw, clip);
    for e in [a, b] {
        draw::line(pm, e.0 - px * t, e.1 - py * t, e.0 + px * t, e.1 + py * t, c, 1.0, lw, clip);
    }
    if let Some((f, pl)) = font.and_then(|f| ruler_plate(a, b, w, f, mm).map(|pl| (f, pl))) {
        if let Some(r) = tiny_skia::Rect::from_xywh(pl.x, pl.y, pl.w, pl.h) {
            if let Some(p) = draw::rounded_rect(r, 5.0) {
                pm.fill_path(&p, &draw::paint(c, 0.92), FillRule::Winding, Transform::identity(), clip);
            }
            let luma = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
            let fg = if luma > 160.0 { [0x11, 0x11, 0x11] } else { [0xff, 0xff, 0xff] };
            draw::draw_text(pm, f, &pl.text, pl.x + pl.pad, pl.y + pl.pad / 2.0, pl.size, fg, 1.0, clip);
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
    let head = arrow_head(w).min(len);
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
