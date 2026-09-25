//! Элементы интерфейса оверлея: панели, иконки, окно стиля, подсказки, лупа, подписи.
//! Вид как у панели инструментов Photoshop: графитовые панели, группы через
//! разделители, выбранный инструмент в тёмной ячейке с синей обводкой.

#[path = "icons.rs"]
pub mod icons;

use crate::draw::{self, Rgb};
use crate::shapes::{self, Tool};
use ab_glyph::FontVec;
use icons::El;
use std::sync::atomic::{AtomicU32, Ordering};
use tiny_skia::{LineCap, PathBuilder, Pixmap, Rect, Stroke, Transform};

pub const PALETTE: [u32; 8] = [
    0xE24B4A, 0xEF9F27, 0xF5D90A, 0x3BB54A, 0x378ADD, 0x7F77DD, 0x111111, 0xFFFFFF,
];

const PANEL: Rgb = [0x36, 0x37, 0x3b];
const BORDER: Rgb = [0x4b, 0x4c, 0x52];
const HEADER: Rgb = [0x2d, 0x2e, 0x32];
const HEADER_LINE: Rgb = [0x22, 0x23, 0x26];
const GRIP: Rgb = [0x6d, 0x6e, 0x74];
const HOVER: Rgb = [0x46, 0x47, 0x4d];
const FG: Rgb = [0xd0, 0xd0, 0xd5];
const MUTED: Rgb = [0xa8, 0xa9, 0xaf];
const DISABLED: Rgb = [0x6a, 0x6b, 0x71];
const SEP: Rgb = [0x4a, 0x4b, 0x51];
const WELL: Rgb = [0x23, 0x24, 0x28];
const TIP_BG: Rgb = [0x1e, 0x1f, 0x22];
const KEY_BORDER: Rgb = [0x55, 0x56, 0x5c];
/// Синий выбранного инструмента и главной кнопки «Копировать».
const SEL: Rgb = [0x3d, 0x8e, 0xe6];
const SEL_HOVER: Rgb = [0x57, 0x9e, 0xf0];
/// Акцент выделения, ручек и рамок (оверлей, закреплённый снимок, история).
pub const ACCENT: Rgb = [0x37, 0x8a, 0xdd];
const WHITE: Rgb = [255, 255, 255];
const COPY_LABEL: &str = "Копировать";

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
    /// Цвет и толщина: плитка внизу панели, клик открывает окно стиля.
    Style,
    Undo,
    Redo,
    Pin,
    CopyText,
    AutoHide,
    Copy,
    Save,
    Close,
    /// Цвет в окне стиля (номер в PALETTE).
    Swatch(usize),
    /// Готовый размер в окне стиля (номер в WidthSpec::presets).
    Preset(usize),
    /// Двойная стрелка в заголовке: одна или две колонки.
    Columns,
    SavePng,
    SaveProject,
    SaveSvg,
}

impl Btn {
    /// Подсказка: название, клавиша, пояснение (пустые строки не показываются).
    pub fn tip(self) -> Option<(&'static str, &'static str, &'static str)> {
        Some(match self {
            Btn::Tool(t) => t.tip(),
            Btn::Style => ("Цвет и толщина", "", "клик: палитра и размеры, колесо мыши: толщина"),
            Btn::Undo => ("Отменить", "Ctrl+Z", ""),
            Btn::Redo => ("Повторить", "Ctrl+Shift+Z", ""),
            Btn::Pin => ("Закрепить поверх окон", "P", ""),
            Btn::CopyText => ("Копировать текст", "Ctrl+Shift+C", "распознать текст на снимке"),
            Btn::AutoHide => ("Скрыть личные данные", "H", "почта, телефоны, карты, ключи"),
            Btn::Copy => ("Копировать", "Ctrl+C, Enter", ""),
            Btn::Save => ("Сохранить…", "Ctrl+S", "PNG, SVG, проект .frost"),
            Btn::Close => ("Закрыть", "Esc", ""),
            Btn::Columns => ("Одна или две колонки", "", "двойной клик по заголовку: панель к выделению"),
            Btn::Swatch(_) | Btn::Preset(_) | Btn::SavePng | Btn::SaveProject | Btn::SaveSvg => return None,
        })
    }

    fn icon(self) -> Option<&'static [El]> {
        Some(match self {
            Btn::Tool(t) => match t {
                Tool::Pointer => icons::POINTER,
                Tool::SelectRect => icons::MARQUEE,
                Tool::SelectLasso => icons::LASSO,
                Tool::Ruler => icons::RULER,
                Tool::Pencil => icons::PENCIL,
                Tool::Marker => icons::MARKER,
                Tool::Line => icons::LINE,
                Tool::Arrow => icons::ARROW,
                Tool::Rect => icons::RECT,
                Tool::FilledRect => icons::RECT_FILLED,
                Tool::Ellipse => icons::ELLIPSE,
                Tool::Counter => icons::COUNTER,
                Tool::Text => icons::TEXT,
                Tool::Pixelate => icons::PIXELATE,
            },
            Btn::AutoHide => icons::AUTO_HIDE,
            Btn::Undo => icons::UNDO,
            Btn::Redo => icons::REDO,
            Btn::CopyText => icons::OCR,
            Btn::Pin => icons::PIN,
            Btn::Copy => icons::COPY,
            Btn::Save => icons::SAVE,
            Btn::Close => icons::CLOSE,
            _ => return None,
        })
    }
}

/// Что значит «толщина» у инструмента: подпись, единица, готовые размеры (значения width).
pub struct WidthSpec {
    pub label: &'static str,
    pub unit: &'static str,
    pub presets: [f32; 5],
}

/// None: у инструмента нет толщины (курсор, выделение, закрашенный прямоугольник).
pub fn width_spec(t: Tool) -> Option<WidthSpec> {
    let (label, unit, presets) = match t {
        Tool::Pointer | Tool::SelectRect | Tool::SelectLasso | Tool::FilledRect => return None,
        Tool::Text => ("Размер текста", "", [2.0, 4.0, 6.0, 9.0, 12.0]),
        Tool::Counter => ("Размер номера", "", [2.0, 4.0, 6.0, 9.0, 12.0]),
        Tool::Pixelate => ("Блок", " px", [2.0, 4.0, 6.0, 10.0, 16.0]),
        Tool::Marker => ("Толщина", " px", [3.0, 4.0, 6.0, 8.0, 10.0]),
        _ => ("Толщина", " px", [2.0, 4.0, 6.0, 10.0, 16.0]),
    };
    Some(WidthSpec { label, unit, presets })
}

/// Число, которое видит пользователь: кегль текста, блок пикселизации, ширина маркера.
pub fn width_value(t: Tool, w: f32) -> f32 {
    match t {
        Tool::Text => shapes::font_size(w),
        Tool::Pixelate => (8.0 + w * 2.0).max(2.0),
        Tool::Marker => shapes::marker_width(w),
        _ => w,
    }
    .round()
}

/// Подпись значения с единицей: «4 px», «26», «Блок 16 px» не нужен, только число.
pub fn width_text(t: Tool, w: f32) -> String {
    let unit = width_spec(t).map_or("", |s| s.unit);
    format!("{}{unit}", width_value(t, w))
}

/// Окно стиля: номер панели и строки подписей (цвет, толщина, подсказка).
pub struct PopRows {
    pub panel: usize,
    pub color: Rect,
    pub width: Option<Rect>,
    pub hint: Option<Rect>,
}

/// Панели в порядке наложения: позже = выше (панель инструментов, действий,
/// меню сохранения, окно стиля). У каждой кнопки своя панель (owner): рисуются
/// вместе, клик достаётся самой верхней панели под курсором.
pub struct Layout {
    pub panels: Vec<Rect>,
    pub buttons: Vec<(Btn, Rect)>,
    /// Номер панели каждой кнопки (параллельно buttons).
    pub owner: Vec<usize>,
    /// Разделители групп: номер панели и линия.
    pub seps: Vec<(usize, Rect)>,
    /// Заголовок панели инструментов: за него панель перетаскивают.
    pub grip: Option<Rect>,
    /// Подсказки панели инструментов ставить справа от неё (там нет выделения).
    pub tips_right: bool,
    pub pop: Option<PopRows>,
}

/// Инструменты по группам, как в Photoshop: выбор и измерение, рисование, фигуры,
/// текст и скрытие, история и стиль. Стиль в две колонки занимает целую строку.
const TOOL_GROUPS: &[&[Btn]] = &[
    &[Btn::Tool(Tool::Pointer), Btn::Tool(Tool::SelectRect), Btn::Tool(Tool::SelectLasso), Btn::Tool(Tool::Ruler)],
    &[Btn::Tool(Tool::Pencil), Btn::Tool(Tool::Marker), Btn::Tool(Tool::Line), Btn::Tool(Tool::Arrow)],
    &[Btn::Tool(Tool::Rect), Btn::Tool(Tool::FilledRect), Btn::Tool(Tool::Ellipse), Btn::Tool(Tool::Counter)],
    &[Btn::Tool(Tool::Text), Btn::Tool(Tool::Pixelate), Btn::AutoHide],
    &[Btn::Undo, Btn::Redo, Btn::Style],
];

impl Layout {
    /// Кнопка на панели в конец наложения (последняя добавленная панель).
    fn add(&mut self, b: Btn, r: Rect) {
        self.buttons.push((b, r));
        self.owner.push(self.panels.len() - 1);
    }
    /// Самая верхняя панель под точкой.
    fn top_panel(&self, x: f32, y: f32) -> Option<usize> {
        self.panels.iter().rposition(|r| contains(r, x, y))
    }
    /// Кнопка под точкой на самой верхней панели: то, что видно, то и нажимается.
    pub fn hit(&self, x: f32, y: f32) -> Option<Btn> {
        let top = self.top_panel(x, y);
        self.buttons
            .iter()
            .zip(&self.owner)
            .rev()
            .find(|((_, r), o)| contains(r, x, y) && top.is_none_or(|t| **o == t))
            .map(|((b, _), _)| *b)
    }
    /// Заголовок панели инструментов под точкой и не закрыт другой панелью.
    pub fn over_grip(&self, x: f32, y: f32) -> bool {
        self.grip.is_some_and(|r| contains(&r, x, y)) && self.top_panel(x, y) == Some(0)
    }
    pub fn over_panel(&self, x: f32, y: f32) -> bool {
        self.panels.iter().any(|r| contains(r, x, y))
    }
    fn rect_of(&self, b: Btn) -> Option<Rect> {
        self.buttons.iter().find(|(x, _)| *x == b).map(|(_, r)| *r)
    }
}

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left() && x < r.right() && y >= r.top() && y < r.bottom()
}

fn ov(a: Rect, b: Rect) -> bool {
    a.left() < b.right() && b.left() < a.right() && a.top() < b.bottom() && b.top() < a.bottom()
}

/// Прямоугольник раскладки: не меньше 1 px по каждой стороне (Rect не бывает пустым).
fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_xywh(x, y, w.max(1.0), h.max(1.0)).unwrap_or(Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap())
}

pub struct LayoutArgs<'a> {
    /// Выделение на экране.
    pub bbox: Rect,
    /// Размер монитора на экране.
    pub mw: f32,
    pub mh: f32,
    pub s: f32,
    pub save_menu: bool,
    /// Открыто окно стиля: для какого инструмента подбирать размеры
    /// (у выбранной фигуры: её инструмент). Some(None): только цвета.
    pub style_pop: Option<Option<Tool>>,
    /// Левый верхний угол панели инструментов, если её перетащили (иначе у выделения).
    pub tools_at: Option<(f32, f32)>,
    /// Колонок в панели инструментов: 1 или 2.
    pub cols: usize,
    pub font: Option<&'a FontVec>,
}

pub fn layout(a: &LayoutArgs) -> Layout {
    let (s, bbox, mw, mh) = (a.s, a.bbox, a.mw, a.mh);
    let b = (32.0 * s).round();
    let pad = (4.0 * s).round();
    let gap = (8.0 * s).round();
    let head = (16.0 * s).round();
    let sep = (9.0 * s).round();
    let cols = a.cols.clamp(1, 2);
    let mut out = Layout { panels: vec![], buttons: vec![], owner: vec![], seps: vec![], grip: None, tips_right: true, pop: None };

    // Панель инструментов: группы по одной-две кнопки в ряд, между группами разделитель.
    // Не помещается по высоте: следующие группы уходят в соседний столбец.
    let mut place: Vec<(usize, f32)> = Vec::new(); // (столбец групп, отступ сверху)
    let (mut gcol, mut y, mut max_h) = (0usize, 0.0f32, 0.0f32);
    for g in TOOL_GROUPS {
        let rows = if cols == 2 { g.len().div_ceil(2) } else { g.len() };
        let gh = rows as f32 * b;
        if y > 0.0 && y + sep + gh > mh - 2.0 * pad - head {
            (gcol, y) = (gcol + 1, 0.0);
        }
        if y > 0.0 {
            y += sep;
        }
        place.push((gcol, y));
        y += gh;
        max_h = max_h.max(y);
    }
    let gcols = gcol + 1;
    let colw = cols as f32 * b;
    let (vw, vh) = (gcols as f32 * colw + (gcols - 1) as f32 * gap + 2.0 * pad, head + max_h + 2.0 * pad);
    let (vx, vy) = match a.tools_at {
        // Перетащили: там и стоит, но целиком на экране.
        Some((x, y)) => (x.clamp(0.0, (mw - vw).max(0.0)), y.clamp(0.0, (mh - vh).max(0.0))),
        None => {
            let mut vx = bbox.right() + gap;
            if vx + vw > mw {
                vx = bbox.left() - gap - vw;
                if vx < 0.0 {
                    vx = bbox.right() - gap - vw;
                }
            }
            (vx, bbox.top().min(mh - vh).max(0.0))
        }
    };
    let vpanel = rect(vx, vy, vw, vh);
    out.panels.push(vpanel);
    out.grip = Some(rect(vx, vy, vw, head));
    out.tips_right = vx + vw / 2.0 >= bbox.left() + bbox.width() / 2.0;
    out.add(Btn::Columns, rect(vx + 2.0 * s, vy + s, 16.0 * s, head - 2.0 * s));
    for (g, (gc, gy)) in TOOL_GROUPS.iter().zip(&place) {
        let gx = vx + pad + *gc as f32 * (colw + gap);
        let first = *gy == 0.0;
        let gy = vy + head + pad + gy;
        if !first {
            let t = (gy - sep / 2.0 - 0.5 * s).round();
            out.seps.push((0, rect(gx + 3.0 * s, t, colw - 6.0 * s, s.max(1.0))));
        }
        for (i, it) in g.iter().enumerate() {
            let (c, r) = if cols == 2 { ((i % 2) as f32, (i / 2) as f32) } else { (0.0, i as f32) };
            let w = if cols == 2 && *it == Btn::Style { 2.0 * b } else { b };
            out.add(*it, rect(gx + c * b, gy + r * b, w, b));
        }
    }

    // Панель действий: распознать, закрепить | Копировать (с подписью), сохранить | закрыть.
    let sepw = (7.0 * s).round();
    let lab = (ui_font() - 3.0) * s;
    let text_w = a.font.map_or(lab * 5.6, |f| draw::text_size(f, COPY_LABEL, lab).0);
    let copy_w = (33.0 * s + text_w + 12.0 * s).round();
    // «Сохранить» чуть шире: справа уголок, там меню (PNG, SVG, проект).
    let save_w = (b + 8.0 * s).round();
    let (hw, hh) = (2.0 * pad + 3.0 * b + save_w + 2.0 * sepw + copy_w, b + 2.0 * pad);
    let mut hy = bbox.bottom() + gap;
    if hy + hh > mh {
        hy = bbox.top() - gap - hh;
        if hy < 0.0 {
            hy = bbox.bottom() - gap - hh;
        }
    }
    let mut hx = (bbox.right() - hw).min(mw - hw).max(0.0);
    let mut hpanel = rect(hx, hy, hw, hh);
    if ov(hpanel, vpanel) {
        hx = if vx - gap - hw >= 0.0 { vx - gap - hw } else { vx + vw + gap };
        hpanel = rect(hx, hy, hw, hh);
    }
    out.panels.push(hpanel);
    let top = hy + pad;
    let vsep = |x: f32| rect((x + sepw / 2.0).round(), top + (b - 20.0 * s) / 2.0, s.max(1.0), 20.0 * s);
    let mut x = hx + pad;
    for it in [Btn::CopyText, Btn::Pin] {
        out.add(it, rect(x, top, b, b));
        x += b;
    }
    out.seps.push((1, vsep(x)));
    x += sepw;
    out.add(Btn::Copy, rect(x, top, copy_w, b));
    x += copy_w;
    out.add(Btn::Save, rect(x, top, save_w, b));
    x += save_w;
    out.seps.push((1, vsep(x)));
    x += sepw;
    out.add(Btn::Close, rect(x, top, b, b));

    // Меню сохранения над (или под) панелью действий.
    if a.save_menu {
        let mh_item = b;
        let items = [Btn::SavePng, Btn::SaveSvg, Btn::SaveProject];
        let mw_ = (300.0 * s * ui_font() / 18.0).round();
        let mhh = items.len() as f32 * mh_item + 2.0 * pad;
        let save_r = out.rect_of(Btn::Save).unwrap();
        let mx = (save_r.right() - mw_).clamp(0.0, (mw - mw_).max(0.0));
        let my = if hpanel.top() - gap - mhh >= 0.0 { hpanel.top() - gap - mhh } else { hpanel.bottom() + gap };
        out.panels.push(rect(mx, my, mw_, mhh));
        for (i, it) in items.iter().enumerate() {
            out.add(*it, rect(mx + pad, my + pad + i as f32 * mh_item, mw_ - 2.0 * pad, mh_item));
        }
    }

    // Окно стиля у плитки: цвета 4×2, готовые размеры, подсказка про колесо.
    if let Some(tool) = a.style_pop {
        let spec = tool.and_then(width_spec);
        let pp = (9.0 * s).round();
        let cell = (29.0 * s).round();
        let row = ((ui_font() - 4.0) * s * 1.5).round();
        let (pre_w, pre_h) = ((23.0 * s).round(), (28.0 * s).round());
        let pw = 4.0 * cell + 2.0 * pp;
        let mut ph = 2.0 * pp + row + 2.0 * cell;
        if spec.is_some() {
            ph += (8.0 * s).round() + row + pre_h + (2.0 * s).round() + row;
        }
        let sr = out.rect_of(Btn::Style).unwrap();
        let right = vpanel.right() + gap;
        let left = vpanel.left() - gap - pw;
        let px = if out.tips_right { if right + pw <= mw { right } else { left } } else if left >= 0.0 { left } else { right };
        let px = px.clamp(0.0, (mw - pw).max(0.0));
        let py = (sr.bottom() + 4.0 * s - ph).clamp(0.0, (mh - ph).max(0.0));
        out.panels.push(rect(px, py, pw, ph));
        let panel = out.panels.len() - 1;
        let (ix, mut iy) = (px + pp, py + pp);
        let color = rect(ix, iy, 4.0 * cell, row);
        iy += row;
        for i in 0..PALETTE.len() {
            let (c, r) = ((i % 4) as f32, (i / 4) as f32);
            out.add(Btn::Swatch(i), rect(ix + c * cell, iy + r * cell, cell, cell));
        }
        iy += 2.0 * cell;
        let (mut width, mut hint) = (None, None);
        if let Some(spec) = spec {
            iy += (8.0 * s).round();
            width = Some(rect(ix, iy, 4.0 * cell, row));
            iy += row;
            let x0 = ix + (4.0 * cell - spec.presets.len() as f32 * pre_w) / 2.0;
            for i in 0..spec.presets.len() {
                out.add(Btn::Preset(i), rect(x0 + i as f32 * pre_w, iy, pre_w, pre_h));
            }
            iy += pre_h + (2.0 * s).round();
            hint = Some(rect(ix, iy, 4.0 * cell, row));
        }
        out.pop = Some(PopRows { panel, color, width, hint });
    }
    out
}

pub struct UiState<'a> {
    pub tool: Tool,
    pub hover: Option<Btn>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub font: Option<&'a FontVec>,
    pub scale: f32,
    pub cols: usize,
    /// Чей стиль в плитке и окне: инструмент (или инструмент выбранной фигуры),
    /// None: без толщины, только цвет.
    pub style_tool: Option<Tool>,
    pub style_width: f32,
    pub style_color: Rgb,
    pub style_open: bool,
}

/// Фон панели: тёмный ободок, светлая рамка, графит внутри.
fn panel_bg(pm: &mut Pixmap, p: Rect, s: f32) {
    let r = 8.0 * s;
    if let Some(o) = Rect::from_ltrb(p.left() - s, p.top() - s, p.right() + s, p.bottom() + s) {
        draw::fill_rounded(pm, o, r + s, [0, 0, 0], 0.5);
    }
    draw::fill_rounded(pm, p, r, BORDER, 1.0);
    if let Some(i) = p.inset(s, s) {
        draw::fill_rounded(pm, i, r - s, PANEL, 1.0);
    }
}

fn stroke_rounded(pm: &mut Pixmap, r: Rect, radius: f32, c: Rgb, alpha: f32, w: f32) {
    if let Some(p) = draw::rounded_rect(r, radius) {
        draw::stroke_path(pm, &p, c, alpha, w, None);
    }
}

/// Заголовок панели инструментов: тёмная полоса со скруглённым верхом и полоска-хваталка.
fn header(pm: &mut Pixmap, g: Rect, s: f32) {
    let r = 7.0 * s;
    if let Some(top) = Rect::from_xywh(g.left() + s, g.top() + s, g.width() - 2.0 * s, g.height() - s + r) {
        draw::fill_rounded(pm, top, r, HEADER, 1.0);
    }
    if let Some(cut) = Rect::from_xywh(g.left() + s, g.bottom(), g.width() - 2.0 * s, r + s) {
        draw::fill_rect(pm, cut, PANEL, 1.0, None);
    }
    if let Some(line) = Rect::from_xywh(g.left() + s, g.bottom() - s.max(1.0), g.width() - 2.0 * s, s.max(1.0)) {
        draw::fill_rect(pm, line, HEADER_LINE, 1.0, None);
    }
    if g.width() >= 60.0 * s {
        if let Some(bar) = Rect::from_xywh(g.left() + g.width() / 2.0 - 9.0 * s, g.top() + g.height() / 2.0 - 1.5 * s, 18.0 * s, 3.0 * s) {
            draw::fill_rounded(pm, bar, 1.5 * s, GRIP, 1.0);
        }
    }
}

pub fn draw_layout(pm: &mut Pixmap, l: &Layout, st: &UiState) {
    let s = st.scale;
    // Панель за панелью, каждая со своими кнопками: верхняя закрывает нижние целиком.
    for (pi, p) in l.panels.iter().enumerate() {
        panel_bg(pm, *p, s);
        if pi == 0 {
            if let Some(g) = l.grip {
                header(pm, g, s);
            }
        }
        for (_, r) in l.seps.iter().filter(|(o, _)| *o == pi) {
            draw::fill_rect(pm, *r, SEP, 1.0, None);
        }
        for ((btn, r), _) in l.buttons.iter().zip(&l.owner).filter(|(_, o)| **o == pi) {
            draw_button(pm, *btn, *r, st);
        }
        if let (Some(pop), Some(font)) = (&l.pop, st.font) {
            if pop.panel == pi {
                pop_texts(pm, font, pop, st);
            }
        }
    }
    if let (Some(h), Some(font)) = (st.hover, st.font) {
        if let (Some(tip), Some(i)) = (h.tip(), l.buttons.iter().position(|(b, _)| *b == h)) {
            let (r, owner) = (l.buttons[i].1, l.owner[i]);
            let place = match owner {
                0 => TipPlace::Side(l.tips_right, l.panels[0]),
                1 => TipPlace::Above(l.panels[1]),
                _ => TipPlace::None,
            };
            tooltip(pm, font, tip, r, place, s);
        }
    }
}

fn draw_button(pm: &mut Pixmap, btn: Btn, r: Rect, st: &UiState) {
    let s = st.scale;
    let hover = st.hover == Some(btn);
    let bg = r.inset(2.0 * s, 2.0 * s).unwrap_or(r);
    let (cx, cy) = (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);
    match btn {
        Btn::Columns => {
            let c = if hover { WHITE } else { MUTED };
            let size = 12.0 * s;
            // Две колонки: «сложить в одну» (стрелки влево), одна: «развернуть».
            icons::draw(pm, icons::COLUMNS, cx - size / 2.0, cy - size / 2.0, size, c, 1.0, st.cols == 2);
            return;
        }
        Btn::Swatch(i) => {
            let c = draw::rgb(PALETTE[i]);
            let cell = r.inset(1.0 * s, 1.0 * s).unwrap_or(r);
            if hover {
                draw::fill_rounded(pm, cell, 6.0 * s, HOVER, 1.0);
            }
            if c == st.style_color {
                stroke_rounded(pm, cell.inset(0.75 * s, 0.75 * s).unwrap_or(cell), 6.0 * s, SEL, 1.0, 1.5 * s);
            }
            let q = 18.0 * s;
            if let Some(sw) = Rect::from_xywh(cx - q / 2.0, cy - q / 2.0, q, q) {
                draw::fill_rounded(pm, sw, 4.0 * s, c, 1.0);
                stroke_rounded(pm, sw, 4.0 * s, WHITE, 0.3, s);
            }
            return;
        }
        Btn::Preset(i) => {
            let Some(t) = st.style_tool else { return };
            let Some(spec) = width_spec(t) else { return };
            let v = spec.presets[i];
            let cell = r.inset(1.0 * s, 1.0 * s).unwrap_or(r);
            if hover {
                draw::fill_rounded(pm, cell, 6.0 * s, HOVER, 1.0);
            }
            if (v - st.style_width).abs() < 0.01 {
                stroke_rounded(pm, cell.inset(0.75 * s, 0.75 * s).unwrap_or(cell), 6.0 * s, SEL, 1.0, 1.5 * s);
            }
            style_glyph(pm, Some(t), v, st.style_color, cx, cy, 18.0 * s, s, st.font);
            return;
        }
        Btn::SavePng | Btn::SaveProject | Btn::SaveSvg => {
            if hover {
                draw::fill_rounded(pm, bg, 6.0 * s, HOVER, 1.0);
            }
            if let Some(font) = st.font {
                let (label, hint) = match btn {
                    Btn::SavePng => ("PNG…", "Ctrl+S"),
                    Btn::SaveSvg => ("SVG…", "для редактора"),
                    _ => ("Проект .frost…", "для доработки"),
                };
                let size = ui_font() * s;
                let ty = r.top() + (r.height() - draw::line_height(font, size)) / 2.0;
                draw::draw_text(pm, font, label, r.left() + 10.0 * s, ty, size, WHITE, 1.0, None);
                let hs = (ui_font() - 3.0) * s;
                let hw = draw::text_size(font, hint, hs).0;
                let hy = r.top() + (r.height() - draw::line_height(font, hs)) / 2.0;
                draw::draw_text(pm, font, hint, r.right() - hw - 10.0 * s, hy, hs, MUTED, 1.0, None);
            }
            return;
        }
        Btn::Copy => {
            // Главное действие: синяя кнопка с подписью.
            draw::fill_rounded(pm, bg, 6.0 * s, if hover { SEL_HOVER } else { SEL }, 1.0);
            let isz = 20.0 * s;
            icons::draw(pm, icons::COPY, r.left() + 7.0 * s, cy - isz / 2.0, isz, WHITE, 1.0, false);
            if let Some(font) = st.font {
                let size = (ui_font() - 3.0) * s;
                let ty = r.top() + (r.height() - draw::line_height(font, size)) / 2.0;
                draw::draw_text(pm, font, COPY_LABEL, r.left() + 33.0 * s, ty, size, WHITE, 1.0, None);
            }
            return;
        }
        _ => {}
    }
    let active = matches!(btn, Btn::Tool(t) if t == st.tool);
    if active {
        draw::fill_rounded(pm, bg, 6.0 * s, WELL, 1.0);
        stroke_rounded(pm, bg.inset(0.75 * s, 0.75 * s).unwrap_or(bg), 5.5 * s, SEL, 1.0, 1.5 * s);
    } else if hover || (btn == Btn::Style && st.style_open) {
        draw::fill_rounded(pm, bg, 6.0 * s, HOVER, 1.0);
    }
    let disabled = matches!(btn, Btn::Undo if !st.can_undo) || matches!(btn, Btn::Redo if !st.can_redo);
    let fg = if disabled {
        DISABLED
    } else if active || hover {
        WHITE
    } else {
        FG
    };
    if btn == Btn::Style {
        style_face(pm, r, st, hover);
        return;
    }
    if let Some(ic) = btn.icon() {
        let size = 22.0 * s;
        if btn == Btn::Save {
            icons::draw(pm, ic, r.left() + 4.0 * s, cy - size / 2.0, size, fg, 1.0, false);
            let cs = 10.0 * s;
            icons::draw(pm, icons::CHEVRON_DOWN, r.right() - cs - 3.0 * s, cy - cs / 2.0 + 0.5 * s, cs, fg, 0.9, false);
        } else {
            icons::draw(pm, ic, cx - size / 2.0, cy - size / 2.0, size, fg, 1.0, false);
        }
    }
}

/// Кружок цвета с тонким светлым ободком (виден и у чёрного на графите).
fn dot(pm: &mut Pixmap, cx: f32, cy: f32, d: f32, c: Rgb, alpha: f32, s: f32) {
    draw::circle(pm, cx, cy, d / 2.0, c, alpha, true, 0.0);
    draw::circle(pm, cx, cy, d / 2.0, WHITE, 0.3, false, s);
}

/// Образец размера для кнопки в одну колонку и готовых размеров: кружок нужной
/// толщины, буква нужного кегля, кружок номера или блоки мозаики.
#[allow(clippy::too_many_arguments)]
fn style_glyph(pm: &mut Pixmap, t: Option<Tool>, w: f32, c: Rgb, cx: f32, cy: f32, room: f32, s: f32, font: Option<&FontVec>) {
    match t {
        None => {
            let q = 16.0 * s;
            if let Some(r) = Rect::from_xywh(cx - q / 2.0, cy - q / 2.0, q, q) {
                draw::fill_rounded(pm, r, 4.0 * s, c, 1.0);
                stroke_rounded(pm, r, 4.0 * s, WHITE, 0.3, s);
            }
        }
        Some(Tool::Text) => {
            if let Some(f) = font {
                let size = (shapes::font_size(w) * 0.42 * s).clamp(8.0 * s, room);
                let (tw, _) = draw::text_size(f, "A", size);
                let top = cy - draw::line_height(f, size) / 2.0;
                draw::draw_text(pm, f, "A", cx - tw / 2.0, top, size, c, 1.0, None);
            }
        }
        Some(Tool::Counter) => dot(pm, cx, cy, ((11.0 + 1.5 * w) * 0.62 * s).clamp(6.0 * s, room), c, 1.0, s),
        Some(Tool::Pixelate) => {
            let q = ((8.0 + 2.0 * w) * 0.28 * s).clamp(2.5 * s, room / 2.0);
            for (dx, dy, a) in [(0.0, 0.0, 0.9), (1.0, 0.0, 0.45), (0.0, 1.0, 0.45), (1.0, 1.0, 0.9)] {
                if let Some(r) = Rect::from_xywh(cx - q + dx * q, cy - q + dy * q, q, q) {
                    draw::fill_rect(pm, r, FG, a, None);
                }
            }
        }
        Some(Tool::Marker) => dot(pm, cx, cy, (shapes::marker_width(w) * 0.55 * s).clamp(5.0 * s, room), c, 0.5, s),
        Some(_) => dot(pm, cx, cy, (w * s).clamp(2.5 * s, room), c, 1.0, s),
    }
}

/// Лицо кнопки стиля. В две колонки: образец (линия нужной толщины текущим цветом,
/// буква, кружок, мозаика) и число справа. В одну колонку: только образец.
fn style_face(pm: &mut Pixmap, r: Rect, st: &UiState, hover: bool) {
    let s = st.scale;
    let (cx, cy) = (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);
    let (t, w, c) = (st.style_tool, st.style_width, st.style_color);
    if r.width() < r.height() * 1.5 {
        style_glyph(pm, t, w, c, cx, cy, 22.0 * s, s, st.font);
        return;
    }
    // Образец слева (30 px), число справа: между ними всегда зазор.
    let x0 = r.left() + 7.0 * s;
    let room = 30.0 * s;
    match t {
        None => {
            if let Some(bar) = Rect::from_xywh(x0 + 2.0 * s, cy - 5.0 * s, r.width() - 18.0 * s, 10.0 * s) {
                draw::fill_rounded(pm, bar, 3.0 * s, c, 1.0);
                stroke_rounded(pm, bar, 3.0 * s, WHITE, 0.3, s);
            }
            return;
        }
        Some(tool @ (Tool::Text | Tool::Counter | Tool::Pixelate)) => style_glyph(pm, Some(tool), w, c, x0 + room / 2.0, cy, 20.0 * s, s, st.font),
        Some(tool) => {
            let (units, alpha) = if tool == Tool::Marker { (shapes::marker_width(w) * 0.5, 0.5) } else { (w, 1.0) };
            let lw = units.clamp(1.5, 10.0) * s;
            draw::line(pm, x0 + lw / 2.0, cy, x0 + room - lw / 2.0, cy, c, alpha, lw, None);
        }
    }
    if let (Some(font), Some(tool)) = (st.font, t) {
        let text = format!("{}", width_value(tool, w));
        let size = (ui_font() - 4.0) * s;
        let (tw, _) = draw::text_size(font, &text, size);
        let top = cy - draw::line_height(font, size) / 2.0;
        draw::draw_text(pm, font, &text, r.right() - 7.0 * s - tw, top, size, if hover { WHITE } else { FG }, 1.0, None);
    }
}

/// Подписи окна стиля: «Цвет», «Толщина 4 px», «или колесом мыши».
fn pop_texts(pm: &mut Pixmap, font: &FontVec, pop: &PopRows, st: &UiState) {
    let s = st.scale;
    let size = (ui_font() - 4.0) * s;
    let row_text = |pm: &mut Pixmap, r: Rect, text: &str, right: Option<&str>| {
        let top = r.top() + (r.height() - draw::line_height(font, size)) / 2.0 - s;
        draw::draw_text(pm, font, text, r.left() + 2.0 * s, top, size, MUTED, 1.0, None);
        if let Some(v) = right {
            let (tw, _) = draw::text_size(font, v, size);
            draw::draw_text(pm, font, v, r.right() - 2.0 * s - tw, top, size, WHITE, 1.0, None);
        }
    };
    row_text(pm, pop.color, "Цвет", None);
    if let (Some(r), Some(t)) = (pop.width, st.style_tool) {
        if let Some(spec) = width_spec(t) {
            row_text(pm, r, spec.label, Some(&width_text(t, st.style_width)));
        }
    }
    if let Some(r) = pop.hint {
        let text = "или колесом мыши";
        let (tw, _) = draw::text_size(font, text, size);
        let top = r.top() + (r.height() - draw::line_height(font, size)) / 2.0;
        draw::draw_text(pm, font, text, r.left() + (r.width() - tw) / 2.0, top, size, [0x8f, 0x90, 0x96], 1.0, None);
    }
}

enum TipPlace {
    /// Сбоку от панели (true: справа), по высоте кнопки.
    Side(bool, Rect),
    /// Над панелью (под ней, если сверху нет места), по центру кнопки.
    Above(Rect),
    None,
}

/// Подсказка: название, клавиша в рамке, ниже мелко пояснение.
fn tooltip(pm: &mut Pixmap, font: &FontVec, (name, key, desc): (&str, &str, &str), anchor: Rect, place: TipPlace, s: f32) {
    let (mw, mh) = (pm.width() as f32, pm.height() as f32);
    let ns = (ui_font() - 2.0) * s;
    let ks = (ui_font() - 5.0) * s;
    let pad = 8.0 * s;
    let (nw, nh) = draw::text_size(font, name, ns);
    let (kw, _) = draw::text_size(font, key, ks);
    let chip = if key.is_empty() { 0.0 } else { 10.0 * s + kw + 8.0 * s };
    let (dw, dh) = if desc.is_empty() { (0.0, 0.0) } else { draw::text_size(font, desc, ks) };
    let w = (nw + chip).max(dw) + 2.0 * pad;
    let h = nh + if desc.is_empty() { 0.0 } else { dh + 2.0 * s } + pad;
    let (mut x, mut y) = match place {
        TipPlace::Side(right, panel) => {
            let r = panel.right() + 6.0 * s;
            let l = panel.left() - 6.0 * s - w;
            let x = if right { if r + w <= mw { r } else { l } } else if l >= 0.0 { l } else { r };
            (x, anchor.top() + (anchor.height() - h) / 2.0)
        }
        TipPlace::Above(panel) => {
            let y = if panel.top() - 6.0 * s - h >= 0.0 { panel.top() - 6.0 * s - h } else { panel.bottom() + 6.0 * s };
            (anchor.left() + anchor.width() / 2.0 - w / 2.0, y)
        }
        TipPlace::None => return,
    };
    x = x.clamp(0.0, (mw - w).max(0.0));
    y = y.clamp(0.0, (mh - h).max(0.0));
    let Some(box_) = Rect::from_xywh(x, y, w, h) else { return };
    draw::fill_rounded(pm, box_, 6.0 * s, BORDER, 1.0);
    if let Some(inner) = box_.inset(s, s) {
        draw::fill_rounded(pm, inner, 5.0 * s, TIP_BG, 1.0);
    }
    let ty = y + pad / 2.0;
    draw::draw_text(pm, font, name, x + pad, ty, ns, WHITE, 1.0, None);
    if !key.is_empty() {
        let kx = x + pad + nw + 10.0 * s;
        let kh = draw::line_height(font, ks);
        let ky = ty + (nh - kh) / 2.0;
        if let Some(kr) = Rect::from_xywh(kx, ky, kw + 8.0 * s, kh) {
            stroke_rounded(pm, kr, 4.0 * s, KEY_BORDER, 1.0, s);
        }
        draw::draw_text(pm, font, key, kx + 4.0 * s, ky, ks, MUTED, 1.0, None);
    }
    if !desc.is_empty() {
        draw::draw_text(pm, font, desc, x + pad, ty + nh + 2.0 * s, ks, MUTED, 1.0, None);
    }
}

/// Круг кисти у курсора (у пикселизации квадрат блока): белый с тёмным ободком,
/// виден на любом фоне. d: диаметр на экране.
pub fn brush_ring(pm: &mut Pixmap, x: f32, y: f32, d: f32, square: bool, s: f32) {
    let d = d.max(4.0 * s);
    let path = if square {
        Rect::from_xywh(x - d / 2.0, y - d / 2.0, d, d).map(PathBuilder::from_rect)
    } else {
        PathBuilder::from_circle(x, y, d / 2.0)
    };
    if let Some(p) = path {
        let outer = Stroke { width: 3.0 * s, line_cap: LineCap::Round, ..Stroke::default() };
        pm.stroke_path(&p, &draw::paint([0, 0, 0], 0.55), &outer, Transform::identity(), None);
        let inner = Stroke { width: 1.5 * s, line_cap: LineCap::Round, ..Stroke::default() };
        pm.stroke_path(&p, &draw::paint(WHITE, 0.95), &inner, Transform::identity(), None);
    }
}

/// Маленькая плашка с числом (размер у курсора после прокрутки колеса).
pub fn chip(pm: &mut Pixmap, font: &FontVec, text: &str, x: f32, y: f32, s: f32) -> Rect {
    let size = (ui_font() - 4.0) * s;
    let (tw, th) = draw::text_size(font, text, size);
    let (w, h) = (tw + 12.0 * s, th + 4.0 * s);
    let (mw, mh) = (pm.width() as f32, pm.height() as f32);
    let (x, y) = (x.clamp(0.0, (mw - w).max(0.0)), y.clamp(0.0, (mh - h).max(0.0)));
    let r = Rect::from_xywh(x, y, w, h).unwrap();
    draw::fill_rounded(pm, r, 6.0 * s, BORDER, 1.0);
    if let Some(i) = r.inset(s, s) {
        draw::fill_rounded(pm, i, 5.0 * s, PANEL, 1.0);
    }
    draw::draw_text(pm, font, text, x + 6.0 * s, y + 2.0 * s, size, WHITE, 1.0, None);
    r
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

/// Плашка с текстом (размер выделения, подсказки внизу экрана).
pub fn label(pm: &mut Pixmap, font: &FontVec, text: &str, x: f32, y: f32, s: f32) -> Rect {
    let size = ui_font() * s;
    let (tw, th) = draw::text_size(font, text, size);
    let pad = 7.0 * s;
    let r = Rect::from_xywh(x, y, tw + 2.0 * pad, th + 6.0 * s).unwrap();
    draw::fill_rounded(pm, r, 6.0 * s, BORDER, 1.0);
    if let Some(i) = r.inset(s, s) {
        draw::fill_rounded(pm, i, 5.0 * s, PANEL, 1.0);
    }
    draw::draw_text(pm, font, text, x + pad, y + 3.0 * s, size, [0xe8, 0xe8, 0xea], 1.0, None);
    r
}

pub fn label_size(font: &FontVec, text: &str, s: f32) -> (f32, f32) {
    let (tw, th) = draw::text_size(font, text, ui_font() * s);
    (tw + 14.0 * s, th + 6.0 * s)
}
