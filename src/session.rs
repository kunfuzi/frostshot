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
    SaveSvg,
    Pin,
    /// Распознать текст выделения и скопировать его.
    CopyText,
    /// Найти и запикселить личные данные в выделении.
    AutoHide,
    /// Выделение закончено (новое, сдвинутое, изменённое): повод для автоскрытия.
    SelectionDone,
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
    /// Перетаскивание фигуры k от точки start. orig: фигура до правки (к ней считается
    /// сдвиг и к ней возвращает отмена); depth: длина стека отмены после записи
    /// состояния (None: ещё не двигали).
    /// redo: ветка повтора на момент первого сдвига, Esc её возвращает.
    ShapeMove { k: usize, start: Pt, orig: Shape, depth: Option<usize>, redo: Vec<Redo> },
    /// Ручка фигуры k (номер как в Shape::handles), от исходной фигуры orig.
    ShapeHandle { k: usize, handle: usize, orig: Shape, depth: Option<usize>, redo: Vec<Redo> },
    /// Панель инструментов тянут за заголовок: смещение курсора от её угла, точка
    /// нажатия; moved: сдвинули дальше порога (дрожь руки не считается).
    Panel { grab: Pt, start: Pt, moved: bool },
}

/// Масштаб и сдвиг вида монитора (Ctrl + колесо). Точка экрана (x, y)
/// соответствует точке снимка (ox + x / z, oy + y / z).
#[derive(Clone, Copy, Debug)]
struct View {
    z: f32,
    ox: f32,
    oy: f32,
}

impl View {
    const ONE: View = View { z: 1.0, ox: 0.0, oy: 0.0 };

    fn to_img(&self, x: f32, y: f32) -> Pt {
        (self.ox + x / self.z, self.oy + y / self.z)
    }

    fn to_scr(&self, x: f32, y: f32) -> Pt {
        ((x - self.ox) * self.z, (y - self.oy) * self.z)
    }

    fn is_one(&self) -> bool {
        self.z == 1.0 && self.ox == 0.0 && self.oy == 0.0
    }

    /// Не показывать пустоту за краем снимка.
    fn clamp(&mut self, w: f32, h: f32) {
        self.ox = self.ox.clamp(0.0, (w - w / self.z).max(0.0));
        self.oy = self.oy.clamp(0.0, (h - h / self.z).max(0.0));
    }
}

struct TextEdit {
    at: Pt,
    text: String,
    /// Правка готового текста: его место в списке фигур и сам исходный текст.
    reedit: Option<(usize, Shape)>,
}

/// Сколько состояний фигур хранить для отмены.
const UNDO_MAX: usize = 200;

/// Шаг повтора: состояние целиком или фигура открытого проекта, снятая Ctrl+Z
/// (без копии всего списка: у проекта из файла их может быть 10 000).
enum Redo {
    State(Vec<Shape>),
    Popped(Shape),
}

/// Серия однотипных правок выбранной фигуры (колесо, стрелки) отменяется разом.
#[derive(Clone, Copy, PartialEq)]
enum Series {
    Wheel,
    Nudge,
}

pub struct Session {
    shots: Vec<MonitorShot>,
    scales: Vec<f32>,
    dimmed: Vec<Pixmap>,
    base: Vec<Pixmap>,
    base_dirty: Vec<bool>,
    frame: Vec<Pixmap>,
    /// Слой снимка (выделение, фигуры) до масштабирования вида.
    work: Vec<Pixmap>,
    views: Vec<View>,
    /// Сдвиг вида средней кнопкой: монитор, точка экрана и смещение в начале.
    pan: Option<(usize, Pt, Pt)>,
    /// Мониторы, которым нужна перерисовка.
    pub dirty: Vec<bool>,
    active: Option<usize>,
    sel: Option<Selection>,
    shapes: Vec<Shape>,
    /// Состояния фигур до изменений (Ctrl+Z) и отменённые (Ctrl+Shift+Z).
    undo_stack: Vec<Vec<Shape>>,
    redo: Vec<Redo>,
    /// Выбранная фигура в режиме выделения.
    picked: Option<usize>,
    /// Последний клик по фигуре: повторный клик по тексту открывает его на правку.
    last_pick: Option<(std::time::Instant, usize)>,
    /// Идущая серия правок: её начало уже записано для отмены.
    series: Option<(Series, usize)>,
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
    /// Когда крутили колесо толщины: плашка с образцом видна WIDTH_HINT.
    width_hint_at: Option<std::time::Instant>,
    /// Строка статуса внизу экрана (распознавание, «Скрыто: …»); None у времени: пока не сменят.
    status: Option<(String, Option<std::time::Instant>)>,
    /// Идёт распознавание текста в фоне.
    pub ocr_busy: bool,
    /// Миллиметров на пиксель у каждого монитора (линейка), None: неизвестно.
    mm: Vec<Option<f32>>,
    /// Панель инструментов перетащили: монитор и левый верхний угол на экране.
    tools_at: Option<(usize, Pt)>,
    /// Монитор, к снимку которого относятся фигуры (их координаты в его пикселях).
    shapes_mon: Option<usize>,
    /// Фигуры открытого проекта: когда записи отмены кончились, Ctrl+Z снимает их по одной.
    pop_loaded: bool,
    /// Области автоскрытия, пришедшие во время переноса фигуры: применятся после него.
    pending_hide: Vec<Rect>,
    /// Последний клик по заголовку панели: двойной возвращает её к выделению.
    grip_click: Option<std::time::Instant>,
}

/// Сколько держать строку статуса.
pub const STATUS_TIME: std::time::Duration = std::time::Duration::from_millis(3500);

/// Сколько показывать образец толщины после прокрутки колеса.
pub const WIDTH_HINT: std::time::Duration = std::time::Duration::from_millis(1200);

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
        let mm = shots
            .iter()
            .map(|s| crate::platform::monitor_mm_per_px(s.x + s.width() as i32 / 2, s.y + s.height() as i32 / 2, s.width()))
            .collect();
        Self {
            base: dimmed.clone(),
            frame: dimmed.clone(),
            work: dimmed.clone(),
            views: vec![View::ONE; n],
            pan: None,
            dimmed,
            shots,
            scales,
            base_dirty: vec![false; n],
            dirty: vec![true; n],
            active: None,
            sel: None,
            shapes: Vec::new(),
            undo_stack: Vec::new(),
            redo: Vec::new(),
            picked: None,
            last_pick: None,
            series: None,
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
            width_hint_at: None,
            status: None,
            ocr_busy: false,
            mm,
            tools_at: None,
            grip_click: None,
            shapes_mon: None,
            pop_loaded: false,
            pending_hide: Vec::new(),
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
        self.work = Vec::new();
    }

    /// Вернуть сессию к показу: буферы заново, временное состояние сброшено, разметка на месте.
    pub fn wake(&mut self) {
        if self.dimmed.len() != self.shots.len() {
            self.dimmed = self.shots.iter().map(|s| dim_pixmap(&s.pixmap, self.dim)).collect();
            self.base = self.dimmed.clone();
            self.frame = self.dimmed.clone();
            self.work = self.dimmed.clone();
        }
        let n = self.shots.len();
        self.views = vec![View::ONE; n];
        self.pan = None;
        self.base_dirty = vec![true; n];
        self.dirty = vec![true; n];
        self.drag = Drag::None;
        self.picked = None;
        self.series = None;
        self.cursor = None;
        self.hover = None;
        self.palette_open = false;
        self.save_menu = false;
        self.mods = Mods::default();
        self.sync();
    }

    /// Проект `.frost`: снимок монитора с выделением, маска, фигуры.
    pub fn to_project(&mut self) -> Result<Vec<u8>, String> {
        let (header, shot) = self.project_parts().ok_or("нет выделения")?;
        crate::project::build(header, &shot)
    }

    /// Заголовок проекта и копия снимка: PNG можно закодировать в фоновом потоке.
    pub fn project_parts(&mut self) -> Option<(crate::project::Header, Pixmap)> {
        self.commit_text();
        let mon = self.active?;
        let sel = self.sel.as_ref()?;
        let shot = self.shots[mon].pixmap.clone();
        let header = crate::project::Header {
            version: crate::project::VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            width: shot.width(),
            height: shot.height(),
            png_len: 0,
            selection: sel.ops.iter().map(crate::project::op_to_dto).collect(),
            shapes: self.shapes.clone(),
            color: self.color,
            line_width: self.width,
            mm_per_px: self.mm[mon],
        };
        Some((header, shot))
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
        s.shapes_mon = Some(0);
        // Масштаб линейки только монитора, где снимали: монитор, где открыли, тут ни при чём.
        // Нет в файле (проекты до 0.1.4): линейка в пикселях.
        s.mm[0] = h.mm_per_px.filter(|k| k.is_finite() && *k > 0.0 && *k < 5.0);
        // Фигуры проекта отменяются по одной (см. undo), без копий списка на каждую.
        s.pop_loaded = true;
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
        self.has_selection() && matches!(self.drag, Drag::None | Drag::Draw(_) | Drag::Panel { .. })
    }

    fn layout(&self) -> Option<Layout> {
        if !self.toolbars_visible() {
            return None;
        }
        let mon = self.active?;
        let b = self.sel.as_ref()?.bbox()?;
        let (w, h) = self.shot_size(mon);
        let v = self.views[mon];
        let (x0, y0) = v.to_scr(b.x as f32, b.y as f32);
        let r = Rect::from_xywh(x0, y0, b.w as f32 * v.z, b.h as f32 * v.z)?;
        let at = self.tools_at.filter(|(m, _)| *m == mon).map(|(_, p)| p);
        Some(ui::layout(r, w as f32, h as f32, self.scales[mon], self.palette_open, self.save_menu, at))
    }

    fn commit_text(&mut self) {
        if let Some(te) = self.text.take() {
            let shape = Shape { kind: Kind::Text { at: te.at, text: te.text }, color: draw::rgb(self.color), width: self.width };
            match te.reedit {
                Some((i, orig)) => {
                    let i = i.min(self.shapes.len());
                    let same = matches!((&shape.kind, &orig.kind), (Kind::Text { text: a, .. }, Kind::Text { text: b, .. }) if a == b)
                        && shape.color == orig.color
                        && shape.width == orig.width;
                    if same {
                        // Открыли и ничего не поменяли: ни шага отмены, ни потери повтора.
                        self.shapes.insert(i, orig);
                    } else {
                        // Шаг отмены: состояние до правки, с исходным текстом на его месте.
                        let mut before = self.shapes.clone();
                        before.insert(i, orig);
                        self.push_undo(before);
                        if shape.is_meaningful() {
                            self.shapes.insert(i, shape);
                        }
                    }
                }
                None if shape.is_meaningful() => self.push_shape(shape),
                None => {}
            }
            self.mark_sel();
        }
    }

    /// Запомнить фигуры перед изменением (для Ctrl+Z); ветка повтора теряет смысл.
    fn snapshot(&mut self) {
        let mut st = self.shapes.clone();
        // Открыт на правку готовый текст (автоскрытие пришло посреди правки): в записи
        // он стоит на своём месте, как до правки.
        if let Some(TextEdit { reedit: Some((i, orig)), .. }) = &self.text {
            st.insert((*i).min(st.len()), orig.clone());
        }
        self.push_undo(st);
    }

    fn push_undo(&mut self, state: Vec<Shape>) {
        self.undo_stack.push(state);
        self.cap_undo();
        self.redo.clear();
        self.series = None;
    }

    fn cap_undo(&mut self) {
        if self.undo_stack.len() > UNDO_MAX {
            let extra = self.undo_stack.len() - UNDO_MAX;
            self.undo_stack.drain(..extra);
        }
    }

    /// Правка выбранной фигуры в серии: записать состояние только в её начале.
    fn snapshot_series(&mut self, kind: Series, k: usize) {
        if self.series != Some((kind, k)) {
            self.snapshot();
            self.series = Some((kind, k));
        }
    }

    fn push_shape(&mut self, s: Shape) {
        self.snapshot();
        self.shapes.push(s);
    }

    /// Открыть готовый текст на правку: стиль текста становится текущим. Шаг отмены
    /// появится только при изменении (commit_text).
    fn reedit_text(&mut self, k: usize) {
        let sh = self.shapes.remove(k);
        if let Kind::Text { at, text } = &sh.kind {
            let c = sh.color;
            self.color = (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
            self.width = sh.width;
            self.text = Some(TextEdit { at: *at, text: text.clone(), reedit: Some((k, sh.clone())) });
        } else {
            self.shapes.insert(k, sh);
        }
        self.picked = None;
        self.mark_sel();
    }

    /// Фигура под точкой снимка (верхняя), только видимая в выделении.
    fn shape_at(&self, mon: usize, p: Pt) -> Option<usize> {
        let sel = self.sel.as_ref()?;
        if !sel.contains(p.0, p.1) {
            return None;
        }
        let tol = 5.0 * self.scales[mon] / self.views[mon].z;
        let mm = self.mm[mon];
        self.shapes.iter().rposition(|s| s.hit(p, tol, self.font.as_deref(), mm))
    }

    /// Ручка выбранной фигуры под точкой снимка.
    fn picked_handle(&self, mon: usize, p: Pt) -> Option<usize> {
        let sh = self.shapes.get(self.picked?)?;
        let grab = 8.0 * self.scales[mon] / self.views[mon].z;
        sh.handles().iter().position(|q| dist(*q, p) <= grab)
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
        match self.redo.pop() {
            Some(Redo::State(next)) => {
                self.undo_stack.push(std::mem::replace(&mut self.shapes, next));
                self.cap_undo();
            }
            // Снятая фигура проекта: вернуть; следующий Ctrl+Z снова снимет её.
            Some(Redo::Popped(s)) => self.shapes.push(s),
            None => {}
        }
        self.picked = None;
        self.series = None;
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
        self.picked = None;
        self.palette_open = false;
        self.mark_sel();
    }

    fn undo(&mut self) {
        match self.text.take() {
            // Новый текст просто отбрасывается.
            Some(TextEdit { reedit: None, .. }) => {}
            // Правка готового текста: вернуть исходный текст на место, стек не трогаем.
            Some(TextEdit { reedit: Some((i, orig)), .. }) => {
                let i = i.min(self.shapes.len());
                self.shapes.insert(i, orig);
            }
            None => {
                if let Some(prev) = self.undo_stack.pop() {
                    self.redo.push(Redo::State(std::mem::replace(&mut self.shapes, prev)));
                } else if self.pop_loaded {
                    // Записи кончились, остались фигуры открытого проекта: по одной с конца.
                    if let Some(last) = self.shapes.pop() {
                        self.redo.push(Redo::Popped(last));
                    }
                }
            }
        }
        self.picked = None;
        self.series = None;
        self.mark_sel();
    }

    fn start_fresh(&mut self, mon: usize) {
        if let Some(old) = self.active {
            self.base_dirty[old] = true;
            self.mark(old);
        }
        let (w, h) = self.shot_size(mon);
        let same = self.active == Some(mon);
        match (&mut self.sel, same) {
            (Some(s), true) => s.clear(),
            _ => self.sel = Some(Selection::new(w, h)),
        }
        self.active = Some(mon);
        // Новая область на том же мониторе: разметка остаётся (она в координатах
        // снимка), вне области её просто не видно, в том числе после отменённой
        // области (Esc). Другой монитор: другие координаты, разметка сбрасывается.
        // Сброс вручную: правая кнопка.
        if self.shapes_mon != Some(mon) {
            self.shapes.clear();
            self.undo_stack.clear();
            self.redo.clear();
            self.pop_loaded = false;
        }
        self.shapes_mon = Some(mon);
        self.picked = None;
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
                if let Some(k) = self.picked {
                    self.snapshot();
                    self.shapes[k].color = draw::rgb(self.color);
                }
            }
            Btn::Undo => self.undo(),
            Btn::Redo => self.redo_last(),
            Btn::Pin => return Action::Pin,
            Btn::CopyText => return Action::CopyText,
            Btn::AutoHide => return Action::AutoHide,
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
            Btn::SaveSvg => {
                self.save_menu = false;
                return Action::SaveSvg;
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

        let jitter = 4.0 * self.scales[mon];
        if let Drag::Panel { grab, start, moved } = &mut self.drag {
            if !*moved && dist((x, y), *start) < jitter {
                return;
            }
            *moved = true;
            self.tools_at = Some((mon, (x - grab.0, y - grab.1)));
            return;
        }

        if let Some((pm, start, o)) = self.pan {
            if pm == mon {
                let (w, h) = self.shot_size(mon);
                let v = &mut self.views[mon];
                v.ox = o.0 - (x - start.0) / v.z;
                v.oy = o.1 - (y - start.1) / v.z;
                v.clamp(w as f32, h as f32);
                self.base_dirty[mon] = true;
                return;
            }
        }

        if self.active == Some(mon) {
            let img = self.views[mon].to_img(x, y);
            let p = self.clamp(mon, img);
            let shift = self.mods.shift;
            // Правка выбранной фигуры: всегда от исходной фигуры (угол можно провести
            // за противоположную сторону), состояние для отмены пишется при первом сдвиге.
            let edit = match &self.drag {
                Drag::ShapeMove { k, start, orig, depth, .. } => {
                    let d = (p.0 - start.0, p.1 - start.1);
                    (depth.is_some() || d != (0.0, 0.0)).then(|| {
                        let mut sh = orig.clone();
                        sh.translate(d.0, d.1);
                        (*k, sh, depth.is_none())
                    })
                }
                Drag::ShapeHandle { k, handle, orig, depth, .. } => {
                    let mut sh = orig.clone();
                    sh.set_handle(*handle, p);
                    Some((*k, sh, depth.is_none()))
                }
                _ => None,
            };
            if let Some((k, sh, first)) = edit {
                if first {
                    let saved = std::mem::take(&mut self.redo);
                    self.snapshot();
                    let n = self.undo_stack.len();
                    if let Drag::ShapeMove { depth, redo, .. } | Drag::ShapeHandle { depth, redo, .. } = &mut self.drag {
                        *depth = Some(n);
                        *redo = saved;
                    }
                }
                if let Some(slot) = self.shapes.get_mut(k) {
                    *slot = sh;
                }
                self.mark_sel();
                return;
            }
            if matches!(self.drag, Drag::ShapeMove { .. } | Drag::ShapeHandle { .. }) {
                return;
            }
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
                        Kind::Ruler(a, b) => *b = if shift { snap45(*a, p) } else { p },
                        Kind::Text { .. } => {}
                    }
                    self.mark_sel();
                }
                Drag::ShapeMove { .. } | Drag::ShapeHandle { .. } | Drag::Panel { .. } => {}
            }
        } else {
            self.hover = None;
        }
    }

    pub fn on_left_press(&mut self, mon: usize, x: f32, y: f32) -> Action {
        self.on_move(mon, x, y);
        let (ix, iy) = self.views[mon].to_img(x, y);
        let p = self.clamp(mon, (ix, iy));

        if self.active == Some(mon) {
            if let Some(l) = self.layout() {
                // Кнопки верхней панели (палитра, меню сохранения поверх остальных) первыми.
                if let Some(b) = l.hit(x, y) {
                    return self.click_btn(b);
                }
                // Заголовок панели: тянуть; двойной клик возвращает панель к выделению.
                if l.over_grip(x, y) {
                    let now = std::time::Instant::now();
                    if self.grip_click.is_some_and(|t| now.duration_since(t) < std::time::Duration::from_millis(400)) {
                        self.grip_click = None;
                        self.tools_at = None;
                        self.mark(mon);
                        return Action::None;
                    }
                    self.grip_click = Some(now);
                    let v = l.panels[0];
                    self.drag = Drag::Panel { grab: (x - v.left(), y - v.top()), start: (x, y), moved: false };
                    return Action::None;
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
        // Курсор (V): только готовые фигуры, выделение не трогает.
        if on_active && self.tool == Tool::Pointer {
            // Выбранная фигура: ручки важнее рамки выделения.
            if let (Some(h), Some(k)) = (self.picked_handle(mon, p), self.picked) {
                self.drag = Drag::ShapeHandle { k, handle: h, orig: self.shapes[k].clone(), depth: None, redo: Vec::new() };
                return Action::None;
            }
            // Клик по фигуре выбирает её; повторный клик по тексту открывает правку.
            if let Some(k) = self.shape_at(mon, p) {
                let now = std::time::Instant::now();
                // Повторный клик: та же фигура всё ещё выбрана с прошлого клика (Esc,
                // Delete, отмена снимают выбор, и старый клик уже не считается).
                let again = self.picked == Some(k)
                    && self.last_pick.is_some_and(|(t, j)| j == k && now.duration_since(t) < std::time::Duration::from_millis(500));
                self.last_pick = Some((now, k));
                if again && matches!(self.shapes[k].kind, Kind::Text { .. }) {
                    self.last_pick = None;
                    self.reedit_text(k);
                    return Action::None;
                }
                self.picked = Some(k);
                self.series = None;
                self.drag = Drag::ShapeMove { k, start: p, orig: self.shapes[k].clone(), depth: None, redo: Vec::new() };
                self.mark_sel();
                return Action::None;
            }
            if self.picked.take().is_some() {
                self.mark_sel();
            }
            return Action::None;
        }

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
                Tool::Ruler => Kind::Ruler(p, p),
                Tool::Text => {
                    let fs = shapes::font_size(w);
                    self.text = Some(TextEdit { at: (p.0, p.1 - fs * 0.6), text: String::new(), reedit: None });
                    self.mark_sel();
                    return Action::None;
                }
                Tool::SelectRect | Tool::SelectLasso | Tool::Pointer => unreachable!(),
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
            let grab = 8.0 * self.scales[mon] / self.views[mon].z;
            if let Some(r) = sel.single_rect() {
                if let Some(h) = handles(r).iter().position(|hp| dist(*hp, (ix, iy)) <= grab) {
                    self.drag = Drag::Resize { handle: h, orig: r };
                    return Action::None;
                }
            }
            // Выделение на весь монитор двигать некуда: протягивание начинает новое.
            let (w, h) = self.shot_size(mon);
            let whole = sel.bbox().is_some_and(|b| b.w == w && b.h == h);
            if sel.contains(ix, iy) && !whole {
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
        let img = self.views[active].to_img(x, y);
        let p = self.clamp(active, img);
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
                self.sync();
                self.mark_sel();
                return Action::SelectionDone;
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
            Drag::Move { .. } | Drag::Resize { .. } => {
                self.sync();
                self.mark_sel();
                return Action::SelectionDone;
            }
            // Панель тянули: следующий клик по заголовку не двойной.
            Drag::Panel { moved: true, .. } => self.grip_click = None,
            Drag::ShapeMove { .. } | Drag::ShapeHandle { .. } => self.apply_pending_hide(),
            Drag::Panel { .. } | Drag::None => {}
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
        // Сначала снять выбор с фигуры.
        if self.picked.take().is_some() {
            self.mark_sel();
            return Action::None;
        }
        if self.active.is_some() {
            let a = self.active.take().unwrap();
            self.sel = None;
            self.shapes.clear();
            self.undo_stack.clear();
            self.redo.clear();
            self.shapes_mon = None;
            self.pop_loaded = false;
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
            // Фигуру уже двигали: вернуть её как была. Запись отмены, сделанную в начале
            // переноса, убрать, только если после неё ничего не записывали (автоскрытие).
            Drag::ShapeMove { k, orig, depth: Some(d), redo, .. } | Drag::ShapeHandle { k, orig, depth: Some(d), redo, .. } => {
                if let Some(slot) = self.shapes.get_mut(k) {
                    *slot = orig;
                }
                if self.undo_stack.len() == d {
                    self.undo_stack.pop();
                    // Переноса не было: и повтор как до него.
                    self.redo = redo;
                }
                self.apply_pending_hide();
            }
            Drag::ShapeMove { .. } | Drag::ShapeHandle { .. } => self.apply_pending_hide(),
            _ => {}
        }
        self.mark_sel();
        Action::None
    }

    /// Автоскрытие, отложенное на время переноса фигуры.
    fn apply_pending_hide(&mut self) {
        if !self.pending_hide.is_empty() {
            let rects = std::mem::take(&mut self.pending_hide);
            self.apply_hide(&rects);
        }
    }

    /// Ctrl + колесо: приблизить вид монитора к точке под курсором (экранные x, y).
    pub fn on_zoom(&mut self, mon: usize, lines: f32, x: f32, y: f32) {
        let (w, h) = self.shot_size(mon);
        let v = &mut self.views[mon];
        let (ix, iy) = v.to_img(x, y);
        let mut z = (v.z * 1.25f32.powf(lines.signum())).clamp(1.0, 16.0);
        if (z - 1.0).abs() < 0.03 {
            z = 1.0;
        }
        v.z = z;
        v.ox = ix - x / z;
        v.oy = iy - y / z;
        v.clamp(w as f32, h as f32);
        self.mark(mon);
    }

    pub fn reset_zoom(&mut self) {
        for (i, v) in self.views.iter_mut().enumerate() {
            if !v.is_one() {
                *v = View::ONE;
                self.dirty[i] = true;
            }
        }
    }

    /// Средняя кнопка: сдвиг увеличенного вида.
    pub fn on_middle(&mut self, mon: usize, pressed: bool, x: f32, y: f32) {
        self.pan = if pressed && self.views[mon].z > 1.0 {
            let v = self.views[mon];
            Some((mon, (x, y), (v.ox, v.oy)))
        } else {
            None
        };
        self.mark(mon);
    }

    /// Когда спрятать образец толщины или строку статуса (для таймера цикла событий).
    pub fn width_hint_deadline(&self) -> Option<std::time::Instant> {
        let hint = self.width_hint_at.map(|t| t + WIDTH_HINT);
        let status = self.status.as_ref().and_then(|(_, t)| t.map(|t| t + STATUS_TIME));
        hint.into_iter().chain(status).min()
    }

    /// Показать строку статуса; sticky: держать, пока не заменят.
    pub fn set_status(&mut self, text: impl Into<String>, sticky: bool) {
        self.status = Some((text.into(), (!sticky).then(std::time::Instant::now)));
        self.mark_all();
    }

    fn mark_all(&mut self) {
        for d in &mut self.dirty {
            *d = true;
        }
    }

    /// Снимок выделения без разметки (для распознавания) и его смещение в координатах снимка.
    pub fn ocr_source(&mut self) -> Option<(Pixmap, (f32, f32))> {
        self.sync();
        let mon = self.active?;
        let b = self.sel.as_ref()?.bbox()?;
        let crop = self.shots[mon].pixmap.clone_rect(tiny_skia::IntRect::from_xywh(b.x as i32, b.y as i32, b.w, b.h)?)?;
        Some((crop, (b.x as f32, b.y as f32)))
    }

    /// Запикселить найденные области (координаты снимка). Каждая область отменяется отдельно.
    pub fn apply_hide(&mut self, rects: &[Rect]) -> usize {
        // Правка открытого на редактирование текста не прерывается: snapshot() сам
        // кладёт исходный текст в записи отмены.
        let c = draw::rgb(self.color);
        // Уже запикселенное повторно не закрываем (автоскрытие после расширения выделения).
        let covered = |r: &Rect, shapes: &[Shape]| {
            shapes.iter().any(|s| match s.kind {
                Kind::Pixelate(a, b) => {
                    let (l, t, rr, bb) = (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
                    let iw = (r.right().min(rr) - r.left().max(l)).max(0.0);
                    let ih = (r.bottom().min(bb) - r.top().max(t)).max(0.0);
                    iw * ih >= 0.8 * r.width() * r.height()
                }
                _ => false,
            })
        };
        // Фигуру тащат: применим после переноса (запись отмены переноса не смешается
        // со скрытием), а сейчас только посчитаем для строки статуса.
        if matches!(self.drag, Drag::ShapeMove { .. } | Drag::ShapeHandle { .. }) {
            self.pending_hide.extend_from_slice(rects);
            return rects.iter().filter(|r| !covered(r, &self.shapes)).count();
        }
        let mut added = 0;
        for r in rects {
            if covered(r, &self.shapes) {
                continue;
            }
            added += 1;
            let kind = Kind::Pixelate((r.left(), r.top()), (r.right(), r.bottom()));
            // Мелкие блоки: текст не читается, но видно, что там было.
            self.push_shape(Shape { kind, color: c, width: 2.0 });
        }
        self.mark_sel();
        added
    }

    /// Время образца или статуса вышло: убрать с экрана.
    pub fn expire_width_hint(&mut self) {
        let now = std::time::Instant::now();
        if self.width_hint_at.is_some_and(|t| now >= t + WIDTH_HINT) {
            self.width_hint_at = None;
            if let Some((m, _)) = self.cursor {
                self.mark(m);
            }
        }
        if self.status.as_ref().is_some_and(|(_, t)| t.is_some_and(|t| now >= t + STATUS_TIME)) {
            self.status = None;
            self.mark_all();
        }
    }

    pub fn on_wheel(&mut self, lines: f32) {
        // Фигуру тащат: колесо подождёт (иначе отмена переноса вернула бы не то).
        if matches!(self.drag, Drag::ShapeMove { .. } | Drag::ShapeHandle { .. }) {
            return;
        }
        // Выбрана фигура: колесо меняет её толщину, серия прокруток отменяется разом.
        // Толщина кисти для новых фигур при этом не меняется. У закрашенного
        // прямоугольника толщины нет: колесо ничего не делает и не показывает.
        if let Some(k) = self.picked {
            if !self.shapes[k].has_width() {
                return;
            }
            self.snapshot_series(Series::Wheel, k);
            let w = (self.shapes[k].width + lines.signum()).clamp(1.0, 40.0);
            self.shapes[k].width = w;
        } else {
            self.width = (self.width + lines.signum()).clamp(1.0, 40.0);
        }
        self.width_hint_at = Some(std::time::Instant::now());
        if let Some((m, _)) = self.cursor {
            self.mark(m);
        }
        self.mark_sel();
    }

    pub fn on_key(&mut self, code: Option<KeyCode>, named: Option<NamedKey>, text: Option<&str>) -> Action {
        // Фигуру тащат мышью: только Esc (вернуть как было), остальное подождёт.
        if matches!(self.drag, Drag::ShapeMove { .. } | Drag::ShapeHandle { .. }) {
            return if named == Some(NamedKey::Escape) { self.cancel_drag() } else { Action::None };
        }
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
                if self.picked.take().is_some() {
                    self.mark_sel();
                    return Action::None;
                }
                return Action::Close;
            }
            Some(NamedKey::Enter) if sel => return Action::Copy,
            _ => {}
        }
        // Выбранная фигура: Delete удаляет, стрелки двигают (с Shift на 10 px).
        if let (Some(k), false) = (self.picked, shortcut_ctrl) {
            let step = if self.mods.shift { 10.0 } else { 1.0 };
            let nudge = match named {
                Some(NamedKey::ArrowLeft) => Some((-step, 0.0)),
                Some(NamedKey::ArrowRight) => Some((step, 0.0)),
                Some(NamedKey::ArrowUp) => Some((0.0, -step)),
                Some(NamedKey::ArrowDown) => Some((0.0, step)),
                _ => None,
            };
            if let Some((dx, dy)) = nudge {
                // Фигура не уезжает с монитора целиком: иначе проект потом не откроется.
                let (w, h) = self.active.map_or((0, 0), |m| self.shot_size(m));
                let mm = self.active.and_then(|m| self.mm[m]);
                let (l, t, r, b) = self.shapes[k].extent(self.font.as_deref(), mm);
                // Шаг не дальше, чем пока фигура касается монитора; обратно всегда можно.
                // Целые пиксели: фигура остаётся на своей сетке.
                let lim = |d: f32, lo: f32, hi: f32| if d < 0.0 { d.max(lo.min(0.0)) } else { d.min(hi.max(0.0)) };
                let dx = lim(dx, -r, w as f32 - l).trunc();
                let dy = lim(dy, -b, h as f32 - t).trunc();
                if dx == 0.0 && dy == 0.0 {
                    return Action::None;
                }
                self.snapshot_series(Series::Nudge, k);
                self.shapes[k].translate(dx, dy);
                self.mark_sel();
                return Action::None;
            }
            if matches!(named, Some(NamedKey::Delete | NamedKey::Backspace)) {
                self.snapshot();
                self.shapes.remove(k);
                self.picked = None;
                self.mark_sel();
                return Action::None;
            }
        }
        let Some(code) = code else { return Action::None };
        if shortcut_ctrl {
            return match code {
                KeyCode::KeyC if sel && self.mods.shift => Action::CopyText,
                KeyCode::KeyC if sel => Action::Copy,
                KeyCode::KeyS if sel && self.mods.shift => Action::QuickSave,
                KeyCode::KeyS if sel => Action::Save,
                // Без выделения разметку не видно: отмена вслепую не нужна.
                KeyCode::KeyZ if sel && self.mods.shift => {
                    self.redo_last();
                    Action::None
                }
                KeyCode::KeyZ if sel => {
                    self.undo();
                    Action::None
                }
                KeyCode::KeyY if sel => {
                    self.redo_last();
                    Action::None
                }
                KeyCode::Digit0 | KeyCode::Numpad0 => {
                    self.reset_zoom();
                    Action::None
                }
                _ => Action::None,
            };
        }
        if code == KeyCode::KeyP && sel {
            return Action::Pin;
        }
        if code == KeyCode::KeyH && sel {
            return Action::AutoHide;
        }
        let tool = match code {
            KeyCode::KeyV => Some(Tool::Pointer),
            KeyCode::KeyM => Some(Tool::SelectRect),
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
            KeyCode::KeyR => Some(Tool::Ruler),
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
        if self.pan.is_some() {
            return CursorIcon::Grabbing;
        }
        match &self.drag {
            Drag::Move { .. } => return CursorIcon::Move,
            Drag::Resize { handle, .. } => return handle_cursor(*handle),
            Drag::NewSel { .. } | Drag::Draw(_) | Drag::ShapeHandle { .. } => return CursorIcon::Crosshair,
            Drag::ShapeMove { .. } => return CursorIcon::Move,
            Drag::Panel { .. } => return CursorIcon::Grabbing,
            Drag::None => {}
        }
        if self.active != Some(mon) || !self.has_selection() {
            return CursorIcon::Crosshair;
        }
        if let Some(l) = self.layout() {
            if l.over_grip(x, y) {
                return CursorIcon::Grab;
            }
            if l.hit(x, y).is_some() {
                return CursorIcon::Pointer;
            }
            if l.over_panel(x, y) {
                return CursorIcon::Default;
            }
        }
        match self.tool {
            Tool::Text => CursorIcon::Text,
            Tool::Pointer => {
                let (ix, iy) = self.views[mon].to_img(x, y);
                if self.picked_handle(mon, (ix, iy)).is_some() {
                    CursorIcon::Crosshair
                } else if self.shape_at(mon, (ix, iy)).is_some() {
                    CursorIcon::Move
                } else {
                    CursorIcon::Default
                }
            }
            t if !t.is_selection() => CursorIcon::Crosshair,
            _ => {
                let sel = self.sel.as_ref().unwrap();
                if self.mods.shift || self.mods.alt {
                    return CursorIcon::Crosshair;
                }
                let v = self.views[mon];
                let (ix, iy) = v.to_img(x, y);
                if let Some(r) = sel.single_rect() {
                    let grab = 8.0 * self.scales[mon] / v.z;
                    if let Some(h) = handles(r).iter().position(|hp| dist(*hp, (ix, iy)) <= grab) {
                        return handle_cursor(h);
                    }
                }
                if sel.contains(ix, iy) { CursorIcon::Move } else { CursorIcon::Crosshair }
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
        let view = self.views[mon];
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

        // 1. Слой снимка: выделение, фигуры, текст, контур. Координаты снимка.
        let work = &mut self.work[mon];
        work.data_mut().copy_from_slice(self.base[mon].data());
        let shot = &self.shots[mon].pixmap;
        let cursor = self.cursor.filter(|(m, _)| *m == mon).map(|(_, p)| p);

        let sel = if is_active { self.sel.as_ref().filter(|s| !s.is_empty()) } else { None };
        if let Some(sel) = sel {
            let clip = Some(sel.mask());
            let mm = self.mm[mon];
            for sh in &self.shapes {
                shapes::render(work, sh, shot, font, clip, mm);
            }
            if let Drag::Draw(sh) = &self.drag {
                shapes::render(work, sh, shot, font, clip, mm);
            }
            if let (Some(te), Some(f)) = (&self.text, font) {
                let size = shapes::font_size(self.width);
                let c = draw::rgb(self.color);
                draw::draw_text(work, f, &te.text, te.at.0, te.at.1, size, c, 1.0, clip);
                let last_line = te.text.rsplit('\n').next().unwrap_or("");
                let (lw, _) = draw::text_size(f, last_line, size);
                let lines = te.text.split('\n').count() as f32;
                let lh = draw::line_height(f, size);
                let cx = te.at.0 + lw + 2.0;
                let cy = te.at.1 + (lines - 1.0) * lh;
                draw::line(work, cx, cy, cx, cy + lh, c, 1.0, (s * 1.5).max(1.0), None);
            }

            // Контур маски "бегущими муравьями".
            let (fw, fh) = (work.width(), work.height());
            let d = work.data_mut();
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
        }

        // 2. Вид: без масштаба слой просто меняется местами с кадром, иначе
        //    увеличивается без сглаживания (видны отдельные пиксели).
        if view.is_one() {
            std::mem::swap(&mut self.work[mon], &mut self.frame[mon]);
        } else {
            let frame = &mut self.frame[mon];
            frame.fill(tiny_skia::Color::BLACK);
            let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Nearest, ..PixmapPaint::default() };
            let t = Transform::from_row(view.z, 0.0, 0.0, view.z, -view.ox * view.z, -view.oy * view.z);
            frame.draw_pixmap(0, 0, self.work[mon].as_ref(), &paint, t, None);
        }

        // 3. Интерфейс поверх, в экранных координатах.
        let frame = &mut self.frame[mon];
        if let Some(sel) = sel {
            if let Some(r) = sel.single_rect() {
                if self.tool.is_selection() && !matches!(self.drag, Drag::NewSel { .. }) {
                    let hs = (4.0 * s).round();
                    for (hx, hy) in handles(r) {
                        let (hx, hy) = view.to_scr(hx, hy);
                        if let Some(hr) = Rect::from_xywh(hx - hs, hy - hs, hs * 2.0, hs * 2.0) {
                            draw::fill_rect(frame, hr, ui::ACCENT, 1.0, None);
                            if let Some(inner) = hr.inset(1.0, 1.0) {
                                draw::fill_rect(frame, inner, [255, 255, 255], 1.0, None);
                            }
                        }
                    }
                }
            }

            // Выбранная фигура: пунктирная рамка и ручки.
            if let (Some(sh), true) = (self.picked.and_then(|k| self.shapes.get(k)), self.tool == Tool::Pointer) {
                let (l, t, r, b) = sh.bounds(font, self.mm[mon]);
                let (x0, y0) = view.to_scr(l, t);
                let (x1, y1) = view.to_scr(r, b);
                let pad = 5.0 * s;
                if let Some(rr) = Rect::from_ltrb(x0 - pad, y0 - pad, x1 + pad, y1 + pad) {
                    let path = tiny_skia::PathBuilder::from_rect(rr);
                    let stroke = tiny_skia::Stroke { width: s.max(1.0), dash: tiny_skia::StrokeDash::new(vec![4.0 * s, 3.0 * s], 0.0), ..Default::default() };
                    frame.stroke_path(&path, &draw::paint(ui::ACCENT, 1.0), &stroke, Transform::identity(), None);
                }
                let hs = (4.0 * s).round();
                for (hx, hy) in sh.handles() {
                    let (hx, hy) = view.to_scr(hx, hy);
                    if let Some(hr) = Rect::from_xywh(hx - hs, hy - hs, hs * 2.0, hs * 2.0) {
                        draw::fill_rect(frame, hr, ui::ACCENT, 1.0, None);
                        if let Some(inner) = hr.inset(1.0, 1.0) {
                            draw::fill_rect(frame, inner, [255, 255, 255], 1.0, None);
                        }
                    }
                }
            }

            if let (Some(f), Some(b)) = (font, sel.bbox()) {
                let text = format!("{} × {}", b.w, b.h);
                let (_, lh) = ui::label_size(f, &text, s);
                let (bx, by) = view.to_scr(b.x as f32, b.y as f32);
                let y = if by >= lh + 4.0 * s { by - lh - 4.0 * s } else { by.max(0.0) + 4.0 * s };
                ui::label(frame, f, &text, bx.max(0.0), y, s);
            }

            if let Some(l) = &layout {
                let over_ui = cursor.is_some_and(|(x, y)| l.over_panel(x, y));
                if let (Some((x, y)), false) = (cursor, over_ui) {
                    if !self.tool.is_selection() && !matches!(self.tool, Tool::Text | Tool::Pointer) && matches!(self.drag, Drag::None) {
                        let r = match self.tool {
                            Tool::Marker => shapes::marker_width(self.width) / 2.0,
                            Tool::Counter => shapes::counter_radius(self.width),
                            _ => self.width / 2.0,
                        };
                        draw::circle(frame, x, y, (r * view.z).max(2.0), draw::rgb(self.color), 1.0, false, 1.0);
                    }
                }
                let st = ui::UiState {
                    tool: self.tool,
                    color: self.color,
                    hover: self.hover,
                    can_undo: !self.undo_stack.is_empty() || self.text.is_some() || (self.pop_loaded && !self.shapes.is_empty()),
                    can_redo: !self.redo.is_empty(),
                    font,
                    scale: s,
                };
                ui::draw_layout(frame, l, &st);

                // Постоянная плашка толщины у панели инструментов, вне выделения.
                let picked = self.picked.and_then(|k| self.shapes.get(k)).filter(|sh| self.tool == Tool::Pointer && sh.has_width());
                let hint = match picked {
                    Some(sh) => Some((Tool::of(&sh.kind), sh.width, sh.color)),
                    None => ui::has_width(self.tool).then(|| (self.tool, self.width, draw::rgb(self.color))),
                };
                if let (Some((t, w, c)), Some(f), Some(bb)) = (hint, font, sel.bbox()) {
                    let (x0, y0) = view.to_scr(bb.x as f32, bb.y as f32);
                    let sr = Rect::from_xywh(x0, y0, bb.w as f32 * view.z, bb.h as f32 * view.z);
                    let (hw, hh) = ui::width_hint_size(f, t, w, view.z, s);
                    let (fw, fh) = (frame.width() as f32, frame.height() as f32);
                    if let Some((hx, hy)) = sr.and_then(|sr| ui::width_hint_spot(l, sr, hw, hh, fw, fh, s)) {
                        ui::draw_width_hint(frame, f, t, w, c, view.z, hx, hy, s);
                    }
                }
            }
        }

        // Образец толщины у курсора после прокрутки колеса.
        if let (Some((x, y)), Some(t0), Some(f)) = (cursor, self.width_hint_at, font) {
            if t0.elapsed() < WIDTH_HINT {
                // Образец выбранной фигуры или кисти.
                let picked = self.picked.and_then(|k| self.shapes.get(k)).filter(|_| self.tool == Tool::Pointer);
                if picked.is_none_or(|sh| sh.has_width()) {
                    let (t, w, c) = picked.map_or((self.tool, self.width, draw::rgb(self.color)), |sh| (Tool::of(&sh.kind), sh.width, sh.color));
                    ui::width_hint(frame, f, t, w, c, view.z, x, y, s);
                }
            }
        }

        let show_magnifier = sel.is_none() || matches!(self.drag, Drag::NewSel { .. } | Drag::Resize { .. });
        if let (Some((x, y)), true, None) = (cursor, show_magnifier, self.pan) {
            let (ix, iy) = view.to_img(x, y);
            ui::magnifier(frame, shot, x, y, ix, iy, s, font);
        }

        if let Some(f) = font {
            if self.active.is_none() {
                let hint = "Выделите область  ·  клик: весь экран  ·  L: лассо  ·  Shift/Alt: добавить/вычесть  ·  Ctrl+колесо: масштаб  ·  Esc: выход";
                let (w, _) = ui::label_size(f, hint, s);
                ui::label(frame, f, hint, ((frame.width() as f32 - w) / 2.0).max(0.0), 16.0 * s, s);
            }
            let mut bottom = frame.height() as f32 - 16.0 * s;
            if !view.is_one() {
                let text = format!("Масштаб {}%  ·  Ctrl+0: 100%  ·  средняя кнопка: сдвиг", (view.z * 100.0).round());
                let (w, h) = ui::label_size(f, &text, s);
                ui::label(frame, f, &text, ((frame.width() as f32 - w) / 2.0).max(0.0), bottom - h, s);
                bottom -= h + 8.0 * s;
            }
            let picked_hint = self.picked.and_then(|k| self.shapes.get(k)).filter(|_| self.tool == Tool::Pointer).map(|sh| {
                let wheel = if sh.has_width() { "  ·  колесо: толщина" } else { "" };
                format!("Тяните фигуру или ручки  ·  стрелки: сдвиг{wheel}  ·  Delete: удалить  ·  Esc: снять выбор")
            });
            let bottom_text = self.status.as_ref().map(|(t, _)| t.clone()).or(picked_hint);
            let bottom_text = bottom_text.as_deref();
            if let (Some(text), true) = (bottom_text, is_active) {
                let (w, h) = ui::label_size(f, text, s);
                ui::label(frame, f, text, ((frame.width() as f32 - w) / 2.0).max(0.0), bottom - h, s);
            }
        }
        &self.frame[mon]
    }

    /// Фигуры и выбранная фигура (для самопроверки).
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }
    pub fn picked(&self) -> Option<usize> {
        self.picked
    }
    /// Панель инструментов на экране (для самопроверки).
    pub fn tools_rect(&self) -> Option<Rect> {
        self.layout().map(|l| l.panels[0])
    }

    /// SVG: снимок картинкой, фигуры векторами (см. svg.rs). keep_outside: и фигуры
    /// целиком вне выделения.
    pub fn to_svg(&mut self, keep_outside: bool) -> Result<String, String> {
        self.commit_text();
        let mon = self.active.ok_or("нет выделения")?;
        let sel = self.sel.as_mut().ok_or("нет выделения")?;
        sel.ensure();
        let b = sel.bbox().ok_or("пустое выделение")?;
        crate::svg::build(&self.shots[mon].pixmap, sel.mask(), b, &self.shapes, self.font.as_deref(), self.mm[mon], keep_outside)
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
            shapes::render(&mut full, s, shot, font, Some(sel.mask()), self.mm[mon]);
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
