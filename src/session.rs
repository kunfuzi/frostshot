//! Состояние одного захвата: выделение, фигуры, инструменты, ввод и отрисовка кадров.

use crate::capture::MonitorShot;
use crate::draw;
use crate::selection::{SelOp, SelShape, Selection};
use crate::shapes::{self, Kind, Pt, Shape, Tool};
use crate::ui::{self, Btn, Layout};
use ab_glyph::FontVec;
use std::sync::Arc;
use tiny_skia::{Pixmap, PixmapPaint, Rect, Transform};
use winit::keyboard::{KeyCode, NamedKey};
use winit::window::CursorIcon;

#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Close,
    Copy,
    Save,
    QuickSave,
    SaveProject,
    Pin,
}

#[derive(Default, Clone, Copy)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

enum Drag {
    None,
    NewSel { add: bool, fresh: bool, lasso: bool, start: Pt, pts: Vec<Pt> },
    Move { last: Pt },
    Resize { handle: usize, orig: Rect },
    Draw(Shape),
}

struct TextEdit {
    at: Pt,
    text: String,
}

pub struct Session {
    shots: Vec<MonitorShot>,
    scales: Vec<f32>,
    dimmed: Vec<Pixmap>,
    base: Vec<Pixmap>,
    base_dirty: Vec<bool>,
    frame: Vec<Pixmap>,
    /// Мониторы, которым нужна перерисовка.
    pub dirty: Vec<bool>,
    active: Option<usize>,
    sel: Option<Selection>,
    shapes: Vec<Shape>,
    /// Отменённые фигуры для повтора (Ctrl+Shift+Z).
    redo: Vec<Shape>,
    tool: Tool,
    pub color: u32,
    pub width: f32,
    drag: Drag,
    cursor: Option<(usize, Pt)>,
    text: Option<TextEdit>,
    palette_open: bool,
    hover: Option<Btn>,
    font: Option<Arc<FontVec>>,
    pub mods: Mods,
    dim: f32,
    save_menu: bool,
    /// Счётчик тянут от цели к номеру (нажали с Shift); иначе от номера к цели.
    counter_target_first: bool,
}

fn dim_pixmap(src: &Pixmap, dim: f32) -> Pixmap {
    let keep = ((1.0 - dim.clamp(0.0, 0.9)) * 256.0) as u32;
    let mut p = src.clone();
    for px in p.data_mut().chunks_exact_mut(4) {
        px[0] = ((px[0] as u32 * keep) >> 8) as u8;
        px[1] = ((px[1] as u32 * keep) >> 8) as u8;
        px[2] = ((px[2] as u32 * keep) >> 8) as u8;
    }
    p
}

fn dist(a: Pt, b: Pt) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn handles(r: Rect) -> [Pt; 8] {
    let (l, t, rr, b) = (r.left(), r.top(), r.right(), r.bottom());
    let (cx, cy) = ((l + rr) / 2.0, (t + b) / 2.0);
    [(l, t), (cx, t), (rr, t), (rr, cy), (rr, b), (cx, b), (l, b), (l, cy)]
}

fn handle_cursor(h: usize) -> CursorIcon {
    match h {
        0 | 4 => CursorIcon::NwseResize,
        2 | 6 => CursorIcon::NeswResize,
        1 | 5 => CursorIcon::NsResize,
        _ => CursorIcon::EwResize,
    }
}

/// Привязка к 45 градусам с Shift.
fn snap45(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    let ang = (dy.atan2(dx) / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
    (a.0 + len * ang.cos(), a.1 + len * ang.sin())
}

fn square(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let m = dx.abs().max(dy.abs());
    (a.0 + m * dx.signum(), a.1 + m * dy.signum())
}

impl Session {
    pub fn new(shots: Vec<MonitorShot>, scales: Vec<f32>, dim: f32, color: u32, width: f32, font: Option<Arc<FontVec>>) -> Self {
        let dimmed: Vec<Pixmap> = shots.iter().map(|s| dim_pixmap(&s.pixmap, dim)).collect();
        let n = shots.len();
        Self {
            base: dimmed.clone(),
            frame: dimmed.clone(),
            dimmed,
            shots,
            scales,
            base_dirty: vec![false; n],
            dirty: vec![true; n],
            active: None,
            sel: None,
            shapes: Vec::new(),
            redo: Vec::new(),
            tool: Tool::SelectRect,
            color,
            width,
            drag: Drag::None,
            cursor: None,
            text: None,
            palette_open: false,
            hover: None,
            font,
            mods: Mods::default(),
            dim,
            save_menu: false,
            counter_target_first: false,
        }
    }

    /// Геометрия снимков в глобальных координатах (x, y, w, h).
    pub fn monitor_rects(&self) -> Vec<(i32, i32, u32, u32)> {
        self.shots.iter().map(|s| (s.x, s.y, s.width(), s.height())).collect()
    }

    /// Монитор с выделением и его прямоугольник.
    pub fn active_rect(&self) -> Option<(i32, i32, u32, u32)> {
        let m = self.active?;
        let s = &self.shots[m];
        Some((s.x, s.y, s.width(), s.height()))
    }

    /// Освободить производные буферы (затемнение, кадры): сессия хранится до повторного открытия.
    pub fn hibernate(&mut self) {
        self.commit_text();
        self.dimmed = Vec::new();
        self.base = Vec::new();
        self.frame = Vec::new();
    }

    /// Вернуть сессию к показу: буферы заново, временное состояние сброшено, разметка на месте.
    pub fn wake(&mut self) {
        if self.dimmed.len() != self.shots.len() {
            self.dimmed = self.shots.iter().map(|s| dim_pixmap(&s.pixmap, self.dim)).collect();
            self.base = self.dimmed.clone();
            self.frame = self.dimmed.clone();
        }
        let n = self.shots.len();
        self.base_dirty = vec![true; n];
        self.dirty = vec![true; n];
        self.drag = Drag::None;
        self.cursor = None;
        self.hover = None;
        self.palette_open = false;
        self.save_menu = false;
        self.mods = Mods::default();
        self.sync();
    }

    /// Проект `.frost`: снимок монитора с выделением, маска, фигуры.
    pub fn to_project(&mut self) -> Result<Vec<u8>, String> {
        self.commit_text();
        let mon = self.active.ok_or("нет выделения")?;
        let sel = self.sel.as_ref().ok_or("нет выделения")?;
        let shot = &self.shots[mon].pixmap;
        let png = shot.encode_png().map_err(|e| e.to_string())?;
        let header = crate::project::Header {
            version: crate::project::VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            width: shot.width(),
            height: shot.height(),
            png_len: png.len(),
            selection: sel.ops.iter().map(crate::project::op_to_dto).collect(),
            shapes: self.shapes.clone(),
            color: self.color,
            line_width: self.width,
        };
        crate::project::encode(&header, &png)
    }

    /// Сессия из проекта; снимок ставится в левый верхний угол монитора (x, y).
    pub fn from_project(bytes: &[u8], x: i32, y: i32, scale: f32, dim: f32, font: Option<Arc<FontVec>>) -> Result<Self, String> {
        let (h, png) = crate::project::decode(bytes)?;
        let pixmap = Pixmap::decode_png(png).map_err(|e| format!("Повреждённый снимок в проекте: {e}"))?;
        if (pixmap.width(), pixmap.height()) != (h.width, h.height) {
            return Err("Повреждённый проект: размер снимка не совпадает".into());
        }
        let (w, hh) = (pixmap.width(), pixmap.height());
        let shot = MonitorShot { x, y, pixmap };
        let mut s = Session::new(vec![shot], vec![scale], dim, h.color, h.line_width, font);
        let mut sel = Selection::new(w, hh);
        sel.ops = h.selection.iter().filter_map(crate::project::dto_to_op).collect();
        sel.touch();
        sel.ensure();
        if sel.is_empty() {
            return Err("В проекте пустое выделение".into());
        }
        s.sel = Some(sel);
        s.active = Some(0);
        s.shapes = h.shapes;
        s.base_dirty = vec![true];
        Ok(s)
    }

    pub fn shot_size(&self, mon: usize) -> (u32, u32) {
        (self.shots[mon].width(), self.shots[mon].height())
    }

    fn mark(&mut self, mon: usize) {
        if let Some(d) = self.dirty.get_mut(mon) {
            *d = true;
        }
    }

    fn mark_sel(&mut self) {
        if let Some(a) = self.active {
            self.base_dirty[a] = true;
            self.mark(a);
        }
    }

    fn has_selection(&self) -> bool {
        self.sel.as_ref().is_some_and(|s| !s.is_empty())
    }

    fn clamp(&self, mon: usize, p: Pt) -> Pt {
        let (w, h) = self.shot_size(mon);
        (p.0.clamp(0.0, w as f32), p.1.clamp(0.0, h as f32))
    }

    fn toolbars_visible(&self) -> bool {
        self.has_selection() && matches!(self.drag, Drag::None | Drag::Draw(_))
    }

    fn layout(&self) -> Option<Layout> {
        if !self.toolbars_visible() {
            return None;
        }
        let mon = self.active?;
        let b = self.sel.as_ref()?.bbox()?;
        let (w, h) = self.shot_size(mon);
        Some(ui::layout(b.rect(), w as f32, h as f32, self.scales[mon], self.palette_open, self.save_menu))
    }

    fn commit_text(&mut self) {
        if let Some(te) = self.text.take() {
            let shape = Shape { kind: Kind::Text { at: te.at, text: te.text }, color: draw::rgb(self.color), width: self.width };
            if shape.is_meaningful() {
                self.push_shape(shape);
            }
            self.mark_sel();
        }
    }

    /// Новая фигура от пользователя: ветка повтора теряет смысл.
    fn push_shape(&mut self, s: Shape) {
        self.shapes.push(s);
        self.redo.clear();
    }

    fn next_counter(&self) -> u32 {
        self.shapes
            .iter()
            .filter_map(|s| match s.kind {
                Kind::Counter { n, .. } => Some(n),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1
    }

    fn redo_last(&mut self) {
        self.commit_text();
        if let Some(s) = self.redo.pop() {
            self.shapes.push(s);
        }
        self.mark_sel();
    }

    /// Левый верхний угол выделения в глобальных координатах и масштаб монитора.
    pub fn selection_origin(&self) -> Option<(i32, i32, f32)> {
        let m = self.active?;
        let b = self.sel.as_ref()?.bbox()?;
        let s = &self.shots[m];
        Some((s.x + b.x as i32, s.y + b.y as i32, self.scales[m]))
    }

    fn set_tool(&mut self, t: Tool) {
        self.commit_text();
        self.tool = t;
        self.palette_open = false;
        self.mark_sel();
    }

    fn undo(&mut self) {
        if self.text.take().is_none() {
            if let Some(s) = self.shapes.pop() {
                self.redo.push(s);
            }
        }
        self.mark_sel();
    }

    fn start_fresh(&mut self, mon: usize) {
        if let Some(old) = self.active {
            self.base_dirty[old] = true;
            self.mark(old);
        }
        let (w, h) = self.shot_size(mon);
        match (&mut self.sel, self.active == Some(mon)) {
            (Some(s), true) => s.clear(),
            _ => self.sel = Some(Selection::new(w, h)),
        }
        self.active = Some(mon);
        self.shapes.clear();
        self.redo.clear();
        self.text = None;
        self.mark_sel();
    }

    fn click_btn(&mut self, b: Btn) -> Action {
        match b {
            Btn::Tool(t) => self.set_tool(t),
            Btn::Color => {
                self.palette_open = !self.palette_open;
                self.save_menu = false;
            }
            Btn::Swatch(i) => {
                self.color = ui::PALETTE[i];
                self.palette_open = false;
            }
            Btn::Undo => self.undo(),
            Btn::Redo => self.redo_last(),
            Btn::Pin => return Action::Pin,
            Btn::Upload => {}
            Btn::Copy => return Action::Copy,
            Btn::Save => {
                self.save_menu = !self.save_menu;
                self.palette_open = false;
            }
            Btn::SavePng => {
                self.save_menu = false;
                return Action::Save;
            }
            Btn::SaveProject => {
                self.save_menu = false;
                return Action::SaveProject;
            }
            Btn::Close => return Action::Close,
        }
        self.mark_sel();
        Action::None
    }

    /// Актуализировать маску и габарит до hit-test панелей.
    fn sync(&mut self) {
        if let Some(sel) = &mut self.sel {
            sel.ensure();
        }
    }

    pub fn on_move(&mut self, mon: usize, x: f32, y: f32) {
        self.sync();
        if let Some((prev, _)) = self.cursor {
            if prev != mon {
                self.mark(prev);
            }
        }
        self.cursor = Some((mon, (x, y)));
        self.mark(mon);

        if self.active == Some(mon) {
            let p = self.clamp(mon, (x, y));
            let shift = self.mods.shift;
            match &mut self.drag {
                Drag::None => {
                    let hover = self.layout().and_then(|l| l.hit(x, y));
                    self.hover = hover;
                }
                Drag::NewSel { add, lasso, start, pts, .. } => {
                    let shape = if *lasso {
                        if pts.last().is_none_or(|l| dist(*l, p) >= 2.0) {
                            pts.push(p);
                        }
                        SelShape::Poly(pts.clone())
                    } else {
                        match draw::rect_ltrb(start.0.round(), start.1.round(), p.0.round(), p.1.round()) {
                            Some(r) if r.width() >= 1.0 && r.height() >= 1.0 => SelShape::Rect(r),
                            _ => SelShape::Poly(vec![]),
                        }
                    };
                    let add = *add;
                    if let Some(sel) = &mut self.sel {
                        sel.preview = Some(SelOp { add, shape });
                        sel.touch();
                    }
                    self.mark_sel();
                }
                Drag::Move { last } => {
                    let (mut dx, mut dy) = (p.0 - last.0, p.1 - last.1);
                    let (w, h) = (self.shots[mon].width() as f32, self.shots[mon].height() as f32);
                    if let Some(sel) = &mut self.sel {
                        if let Some(b) = sel.bbox() {
                            dx = dx.clamp(-(b.x as f32), w - (b.x + b.w) as f32).round();
                            dy = dy.clamp(-(b.y as f32), h - (b.y + b.h) as f32).round();
                        }
                        sel.translate(dx, dy);
                        sel.ensure();
                    }
                    *last = (last.0 + dx, last.1 + dy);
                    self.mark_sel();
                }
                Drag::Resize { handle, orig } => {
                    let (mut l, mut t, mut r, mut b) = (orig.left(), orig.top(), orig.right(), orig.bottom());
                    let (px, py) = (p.0.round(), p.1.round());
                    match *handle {
                        0 => (l, t) = (px, py),
                        1 => t = py,
                        2 => (r, t) = (px, py),
                        3 => r = px,
                        4 => (r, b) = (px, py),
                        5 => b = py,
                        6 => (l, b) = (px, py),
                        _ => l = px,
                    }
                    if let (Some(nr), Some(sel)) = (draw::rect_ltrb(l, t, r, b), &mut self.sel) {
                        if nr.width() >= 1.0 && nr.height() >= 1.0 {
                            sel.ops = vec![SelOp { add: true, shape: SelShape::Rect(nr) }];
                            sel.touch();
                        }
                    }
                    self.mark_sel();
                }
                Drag::Draw(shape) => {
                    match &mut shape.kind {
                        Kind::Pencil(pts) | Kind::Marker(pts) => {
                            if pts.last().is_none_or(|l| dist(*l, p) >= 1.0) {
                                pts.push(p);
                            }
                        }
                        Kind::Line(a, b) | Kind::Arrow(a, b) => *b = if shift { snap45(*a, p) } else { p },
                        Kind::Rect(a, b) | Kind::FilledRect(a, b) | Kind::Ellipse(a, b) => *b = if shift { square(*a, p) } else { p },
                        Kind::Counter { at, tip: Some(t), .. } => {
                            if self.counter_target_first {
                                *at = p;
                            } else {
                                *t = p;
                            }
                        }
                        Kind::Counter { .. } => {}
                        Kind::Pixelate(_, b) => *b = p,
                        Kind::Text { .. } => {}
                    }
                    self.mark_sel();
                }
            }
        } else {
            self.hover = None;
        }
    }

    pub fn on_left_press(&mut self, mon: usize, x: f32, y: f32) -> Action {
        self.on_move(mon, x, y);
        let p = self.clamp(mon, (x, y));

        if self.active == Some(mon) {
            if let Some(l) = self.layout() {
                if let Some(b) = l.hit(x, y) {
                    return self.click_btn(b);
                }
                if l.over_panel(x, y) {
                    return Action::None;
                }
            }
        }
        if self.palette_open || self.save_menu {
            self.palette_open = false;
            self.save_menu = false;
            self.mark_sel();
        }
        self.commit_text();

        let on_active = self.active == Some(mon) && self.has_selection();
        if on_active && !self.tool.is_selection() {
            let (c, w) = (draw::rgb(self.color), self.width);
            let kind = match self.tool {
                Tool::Pencil => Kind::Pencil(vec![p]),
                Tool::Marker => Kind::Marker(vec![p]),
                Tool::Line => Kind::Line(p, p),
                Tool::Arrow => Kind::Arrow(p, p),
                Tool::Rect => Kind::Rect(p, p),
                Tool::FilledRect => Kind::FilledRect(p, p),
                Tool::Ellipse => Kind::Ellipse(p, p),
                // Выноска-клин: кружок в точке нажатия, клин тянется к цели.
                // С Shift наоборот: нажатие отмечает цель, кружок встанет там, где отпустят.
                Tool::Counter => {
                    self.counter_target_first = self.mods.shift;
                    Kind::Counter { at: p, n: self.next_counter(), tip: Some(p) }
                }
                Tool::Pixelate => Kind::Pixelate(p, p),
                Tool::Text => {
                    let fs = shapes::font_size(w);
                    self.text = Some(TextEdit { at: (p.0, p.1 - fs * 0.6), text: String::new() });
                    self.mark_sel();
                    return Action::None;
                }
                Tool::SelectRect | Tool::SelectLasso => unreachable!(),
            };
            self.drag = Drag::Draw(Shape { kind, color: c, width: w });
            self.mark_sel();
            return Action::None;
        }

        let lasso = self.tool == Tool::SelectLasso;
        if on_active {
            if self.mods.shift || self.mods.alt {
                self.drag = Drag::NewSel { add: !self.mods.alt, fresh: false, lasso, start: p, pts: vec![p] };
                return Action::None;
            }
            let sel = self.sel.as_ref().unwrap();
            let grab = 8.0 * self.scales[mon];
            if let Some(r) = sel.single_rect() {
                if let Some(h) = handles(r).iter().position(|hp| dist(*hp, (x, y)) <= grab) {
                    self.drag = Drag::Resize { handle: h, orig: r };
                    return Action::None;
                }
            }
            // Выделение на весь монитор двигать некуда: протягивание начинает новое.
            let (w, h) = self.shot_size(mon);
            let whole = sel.bbox().is_some_and(|b| b.w == w && b.h == h);
            if sel.contains(x, y) && !whole {
                self.drag = Drag::Move { last: p };
                return Action::None;
            }
        }
        self.start_fresh(mon);
        self.drag = Drag::NewSel { add: true, fresh: true, lasso, start: p, pts: vec![p] };
        Action::None
    }

    pub fn on_left_release(&mut self, mon: usize, x: f32, y: f32) -> Action {
        let Some(active) = self.active else {
            self.drag = Drag::None;
            return Action::None;
        };
        let p = self.clamp(active, (x, y));
        let _ = mon;
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::NewSel { add, fresh, lasso, start, pts } => {
                let (w, h) = self.shot_size(active);
                let sel = self.sel.as_mut().unwrap();
                sel.preview = None;
                let moved = if lasso { pts.iter().any(|q| dist(*q, start) >= 4.0) } else { dist(start, p) >= 4.0 };
                if fresh && !moved {
                    let r = Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap();
                    sel.ops.push(SelOp { add: true, shape: SelShape::Rect(r) });
                } else if moved {
                    let shape = if lasso {
                        SelShape::Poly(pts)
                    } else {
                        match draw::rect_ltrb(start.0.round(), start.1.round(), p.0.round(), p.1.round()) {
                            Some(r) => SelShape::Rect(r),
                            None => SelShape::Poly(vec![]),
                        }
                    };
                    sel.ops.push(SelOp { add, shape });
                }
                sel.touch();
                sel.ensure();
                if sel.is_empty() {
                    self.sel = None;
                    self.base_dirty[active] = true;
                    self.mark(active);
                    self.active = None;
                    return Action::None;
                }
            }
            Drag::Draw(mut shape) => {
                // Счётчик без протягивания: обычный кружок в точке клика.
                if let Kind::Counter { at, tip: tip @ Some(_), .. } = &mut shape.kind {
                    let t = tip.unwrap();
                    if dist(*at, t) < shapes::counter_radius(shape.width) {
                        // В режиме «сначала цель» кружок встаёт в точку клика.
                        if self.counter_target_first {
                            *at = t;
                        }
                        *tip = None;
                    }
                }
                if shape.is_meaningful() {
                    self.push_shape(shape);
                }
            }
            Drag::Move { .. } | Drag::Resize { .. } | Drag::None => {}
        }
        self.sync();
        self.mark_sel();
        Action::None
    }

    /// Правая кнопка: сбросить выделение, без выделения закрыть.
    pub fn on_right_press(&mut self) -> Action {
        if !matches!(self.drag, Drag::None) {
            return self.cancel_drag();
        }
        if self.active.is_some() {
            let a = self.active.take().unwrap();
            self.sel = None;
            self.shapes.clear();
            self.redo.clear();
            self.text = None;
            self.base_dirty[a] = true;
            self.mark(a);
            return Action::None;
        }
        Action::Close
    }

    fn cancel_drag(&mut self) -> Action {
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::NewSel { fresh, .. } => {
                if let Some(sel) = &mut self.sel {
                    sel.preview = None;
                    sel.touch();
                }
                if fresh {
                    if let Some(a) = self.active.take() {
                        self.base_dirty[a] = true;
                        self.mark(a);
                    }
                    self.sel = None;
                }
            }
            Drag::Resize { orig, .. } => {
                if let Some(sel) = &mut self.sel {
                    sel.ops = vec![SelOp { add: true, shape: SelShape::Rect(orig) }];
                    sel.touch();
                }
            }
            _ => {}
        }
        self.mark_sel();
        Action::None
    }

    pub fn on_wheel(&mut self, lines: f32) {
        self.width = (self.width + lines.signum()).clamp(1.0, 40.0);
        if let Some((m, _)) = self.cursor {
            self.mark(m);
        }
        self.mark_sel();
    }

    pub fn on_key(&mut self, code: Option<KeyCode>, named: Option<NamedKey>, text: Option<&str>) -> Action {
        // AltGr приходит как Ctrl+Alt: это ввод символа, а не шорткат.
        let shortcut_ctrl = self.mods.ctrl && !self.mods.alt;
        if let Some(te) = &mut self.text {
            if !shortcut_ctrl {
                match named {
                    Some(NamedKey::Escape) => self.commit_text(),
                    Some(NamedKey::Enter) => te.text.push('\n'),
                    Some(NamedKey::Backspace) => {
                        te.text.pop();
                    }
                    _ => {
                        if let Some(t) = text {
                            te.text.extend(t.chars().filter(|c| !c.is_control()));
                        }
                    }
                }
                self.mark_sel();
                return Action::None;
            }
        }
        let sel = self.has_selection();
        match named {
            Some(NamedKey::Escape) => {
                if !matches!(self.drag, Drag::None) {
                    return self.cancel_drag();
                }
                if self.palette_open || self.save_menu {
                    self.palette_open = false;
                    self.save_menu = false;
                    self.mark_sel();
                    return Action::None;
                }
                return Action::Close;
            }
            Some(NamedKey::Enter) if sel => return Action::Copy,
            _ => {}
        }
        let Some(code) = code else { return Action::None };
        if shortcut_ctrl {
            return match code {
                KeyCode::KeyC if sel => Action::Copy,
                KeyCode::KeyS if sel && self.mods.shift => Action::QuickSave,
                KeyCode::KeyS if sel => Action::Save,
                KeyCode::KeyZ if self.mods.shift => {
                    self.redo_last();
                    Action::None
                }
                KeyCode::KeyZ => {
                    self.undo();
                    Action::None
                }
                KeyCode::KeyY => {
                    self.redo_last();
                    Action::None
                }
                _ => Action::None,
            };
        }
        if code == KeyCode::KeyP && sel {
            return Action::Pin;
        }
        let tool = match code {
            KeyCode::KeyV => Some(Tool::SelectRect),
            KeyCode::KeyL => Some(Tool::SelectLasso),
            KeyCode::Digit1 => Some(Tool::Pencil),
            KeyCode::Digit2 => Some(Tool::Marker),
            KeyCode::Digit3 => Some(Tool::Line),
            KeyCode::Digit4 => Some(Tool::Arrow),
            KeyCode::Digit5 => Some(Tool::Rect),
            KeyCode::Digit6 => Some(Tool::Text),
            KeyCode::Digit7 => Some(Tool::Pixelate),
            KeyCode::Digit8 => Some(Tool::FilledRect),
            KeyCode::Digit9 => Some(Tool::Ellipse),
            KeyCode::Digit0 => Some(Tool::Counter),
            _ => None,
        };
        if let Some(t) = tool {
            self.set_tool(t);
        }
        Action::None
    }

    pub fn cursor_icon(&self, mon: usize) -> CursorIcon {
        let Some((m, (x, y))) = self.cursor else { return CursorIcon::Crosshair };
        if m != mon {
            return CursorIcon::Crosshair;
        }
        match &self.drag {
            Drag::Move { .. } => return CursorIcon::Move,
            Drag::Resize { handle, .. } => return handle_cursor(*handle),
            Drag::NewSel { .. } | Drag::Draw(_) => return CursorIcon::Crosshair,
            Drag::None => {}
        }
        if self.active != Some(mon) || !self.has_selection() {
            return CursorIcon::Crosshair;
        }
        if let Some(l) = self.layout() {
            if l.hit(x, y).is_some() {
                return CursorIcon::Pointer;
            }
            if l.over_panel(x, y) {
                return CursorIcon::Default;
            }
        }
        match self.tool {
            Tool::Text => CursorIcon::Text,
            t if !t.is_selection() => CursorIcon::Crosshair,
            _ => {
                let sel = self.sel.as_ref().unwrap();
                if self.mods.shift || self.mods.alt {
                    return CursorIcon::Crosshair;
                }
                if let Some(r) = sel.single_rect() {
                    let grab = 8.0 * self.scales[mon];
                    if let Some(h) = handles(r).iter().position(|hp| dist(*hp, (x, y)) <= grab) {
                        return handle_cursor(h);
                    }
                }
                if sel.contains(x, y) { CursorIcon::Move } else { CursorIcon::Crosshair }
            }
        }
    }

    /// Кадр монитора для вывода на экран.
    pub fn render(&mut self, mon: usize) -> &Pixmap {
        self.dirty[mon] = false;
        let is_active = self.active == Some(mon);
        if is_active {
            if let Some(sel) = &mut self.sel {
                sel.ensure();
            }
        }
        let layout = if is_active { self.layout() } else { None };
        let s = self.scales[mon];
        let font = self.font.clone();
        let font = font.as_deref();

        if self.base_dirty[mon] {
            self.base_dirty[mon] = false;
            let base = &mut self.base[mon];
            base.data_mut().copy_from_slice(self.dimmed[mon].data());
            if let (true, Some(sel)) = (is_active, &self.sel) {
                if !sel.is_empty() {
                    base.draw_pixmap(0, 0, self.shots[mon].pixmap.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(sel.mask()));
                }
            }
        }

        let frame = &mut self.frame[mon];
        frame.data_mut().copy_from_slice(self.base[mon].data());
        let shot = &self.shots[mon].pixmap;
        let cursor = self.cursor.filter(|(m, _)| *m == mon).map(|(_, p)| p);

        let sel = if is_active { self.sel.as_ref().filter(|s| !s.is_empty()) } else { None };
        if let Some(sel) = sel {
            let clip = Some(sel.mask());
            for sh in &self.shapes {
                shapes::render(frame, sh, shot, font, clip);
            }
            if let Drag::Draw(sh) = &self.drag {
                shapes::render(frame, sh, shot, font, clip);
            }
            if let (Some(te), Some(f)) = (&self.text, font) {
                let size = shapes::font_size(self.width);
                let c = draw::rgb(self.color);
                draw::draw_text(frame, f, &te.text, te.at.0, te.at.1, size, c, 1.0, clip);
                let last_line = te.text.rsplit('\n').next().unwrap_or("");
                let (lw, _) = draw::text_size(f, last_line, size);
                let lines = te.text.split('\n').count() as f32;
                let lh = draw::line_height(f, size);
                let cx = te.at.0 + lw + 2.0;
                let cy = te.at.1 + (lines - 1.0) * lh;
                draw::line(frame, cx, cy, cx, cy + lh, c, 1.0, (s * 1.5).max(1.0), None);
            }

            // Контур маски "бегущими муравьями".
            let (fw, fh) = (frame.width(), frame.height());
            let d = frame.data_mut();
            for &(x, y) in sel.edges() {
                if x >= fw || y >= fh {
                    continue;
                }
                let i = ((y * fw + x) * 4) as usize;
                let c = if ((x + y) / 4) % 2 == 0 { [255, 255, 255] } else { ui::ACCENT };
                d[i] = c[0];
                d[i + 1] = c[1];
                d[i + 2] = c[2];
                d[i + 3] = 255;
            }

            if let Some(r) = sel.single_rect() {
                if self.tool.is_selection() && !matches!(self.drag, Drag::NewSel { .. }) {
                    let hs = (4.0 * s).round();
                    for (hx, hy) in handles(r) {
                        if let Some(hr) = Rect::from_xywh(hx - hs, hy - hs, hs * 2.0, hs * 2.0) {
                            draw::fill_rect(frame, hr, ui::ACCENT, 1.0, None);
                            if let Some(inner) = hr.inset(1.0, 1.0) {
                                draw::fill_rect(frame, inner, [255, 255, 255], 1.0, None);
                            }
                        }
                    }
                }
            }

            if let (Some(f), Some(b)) = (font, sel.bbox()) {
                let text = format!("{} × {}", b.w, b.h);
                let (_, lh) = ui::label_size(f, &text, s);
                let y = if b.y as f32 >= lh + 4.0 * s { b.y as f32 - lh - 4.0 * s } else { b.y as f32 + 4.0 * s };
                ui::label(frame, f, &text, b.x as f32, y, s);
            }

            if let Some(l) = &layout {
                let over_ui = cursor.is_some_and(|(x, y)| l.over_panel(x, y));
                if let (Some((x, y)), false) = (cursor, over_ui) {
                    if !self.tool.is_selection() && self.tool != Tool::Text && matches!(self.drag, Drag::None) {
                        let r = match self.tool {
                            Tool::Marker => shapes::marker_width(self.width) / 2.0,
                            Tool::Counter => shapes::counter_radius(self.width),
                            _ => self.width / 2.0,
                        };
                        draw::circle(frame, x, y, r.max(2.0), draw::rgb(self.color), 1.0, false, 1.0);
                    }
                }
                let st = ui::UiState {
                    tool: self.tool,
                    color: self.color,
                    hover: self.hover,
                    can_undo: !self.shapes.is_empty() || self.text.is_some(),
                    can_redo: !self.redo.is_empty(),
                    font,
                    scale: s,
                };
                ui::draw_layout(frame, l, &st);
            }
        }

        let show_magnifier = sel.is_none() || matches!(self.drag, Drag::NewSel { .. } | Drag::Resize { .. });
        if let (Some((x, y)), true) = (cursor, show_magnifier) {
            ui::magnifier(frame, shot, x, y, s, font);
        }

        if self.active.is_none() {
            if let Some(f) = font {
                let hint = "Выделите область  ·  клик: весь экран  ·  L: лассо  ·  Shift/Alt: добавить/вычесть  ·  Esc: выход";
                let (w, _) = ui::label_size(f, hint, s);
                ui::label(frame, f, hint, ((frame.width() as f32 - w) / 2.0).max(0.0), 16.0 * s, s);
            }
        }
        &self.frame[mon]
    }

    /// Итоговое изображение: габарит выделения, вне маски прозрачно.
    pub fn result(&mut self) -> Option<Pixmap> {
        self.commit_text();
        let mon = self.active?;
        let sel = self.sel.as_mut()?;
        sel.ensure();
        let b = sel.bbox()?;
        let shot = &self.shots[mon].pixmap;
        let mut full = shot.clone();
        let font = self.font.as_deref();
        for s in &self.shapes {
            shapes::render(&mut full, s, shot, font, Some(sel.mask()));
        }
        let mut out = Pixmap::new(b.w, b.h)?;
        let fw = full.width();
        let m = sel.mask().data();
        let fd = full.data();
        let od = out.data_mut();
        for y in 0..b.h {
            for x in 0..b.w {
                let mi = ((b.y + y) * fw + b.x + x) as usize;
                let mv = m[mi] as u32;
                let si = mi * 4;
                let di = ((y * b.w + x) * 4) as usize;
                for c in 0..4 {
                    od[di + c] = (fd[si + c] as u32 * mv / 255) as u8;
                }
            }
        }
        Some(out)
    }
}
