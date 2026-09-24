//! Панель истории у значка в трее: карточки снимков выезжают снизу, как стопка
//! «Загрузки» в Dock на macOS. Своё окно без фокуса; клик по карточке открывает снимок.

use crate::draw::{self, Rgb};
use ab_glyph::FontVec;
use softbuffer::Surface;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Rect, Transform};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event_loop::ActiveEventLoop;
use winit::window::{CursorIcon, Window, WindowLevel};

const BG: Rgb = [0x1b, 0x1b, 0x1e];
const CARD: Rgb = [0x2a, 0x2a, 0x2f];
const CARD_HOVER: Rgb = [0x36, 0x36, 0x3d];
const BORDER: Rgb = [0x44, 0x44, 0x4b];
const FG: Rgb = [0xe6, 0xe6, 0xe6];
const MUTED: Rgb = [0x9a, 0x9a, 0xa2];
const ACCENT: Rgb = crate::ui::ACCENT;

/// Размеры в логических пикселях.
const THUMB_W: f32 = 200.0;
const THUMB_H: f32 = 120.0;
const PAD: f32 = 10.0;
const LABEL_H: f32 = 24.0;
const HEADER_H: f32 = 40.0;
const ANIM: Duration = Duration::from_millis(320);

struct Card {
    path: PathBuf,
    label: String,
    thumb: Option<Pixmap>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    Card(usize),
    Copy(usize),
    Delete(usize),
    Folder,
    Clear,
}

impl Target {
    /// Карточка, к которой относится цель (кнопки на ней держат подсветку карточки).
    fn card(self) -> Option<usize> {
        match self {
            Target::Card(i) | Target::Copy(i) | Target::Delete(i) => Some(i),
            _ => None,
        }
    }
}

pub enum PopupClick {
    None,
    Open(PathBuf),
    Copy(PathBuf),
    Delete(PathBuf),
    Folder,
    Clear,
}

pub struct HistoryPopup {
    pub window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    pm: Pixmap,
    cards: Vec<Card>,
    cols: usize,
    rects: Vec<(Target, Rect)>,
    hover: Option<Target>,
    cursor: (f32, f32),
    opened: Instant,
    font: Option<Arc<FontVec>>,
    /// Рабочая область монитора и центр значка по X: панель встаёт над значком.
    area: (i32, i32, i32, i32),
    anchor_x: i32,
}

fn logical_size(n: usize) -> (usize, f64, f64) {
    let cols = if n > 4 { 2 } else { 1 };
    let rows = n.div_ceil(cols).max(1);
    let card_h = THUMB_H + LABEL_H + PAD;
    let w = cols as f32 * (THUMB_W + 2.0 * PAD) + PAD;
    let h = HEADER_H + rows as f32 * (card_h + PAD) + PAD;
    (cols, w as f64, h as f64)
}

impl HistoryPopup {
    pub fn open(
        el: &ActiveEventLoop,
        font: Option<Arc<FontVec>>,
        entries: &[crate::history::Entry],
        area: (i32, i32, i32, i32),
        anchor_x: i32,
        scale: f32,
    ) -> Result<Self, String> {
        let cards: Vec<Card> = entries
            .iter()
            .map(|e| Card {
                path: e.path.clone(),
                label: e.label.clone(),
                thumb: std::fs::read(&e.thumb).ok().and_then(|b| Pixmap::decode_png(&b).ok()),
            })
            .collect();
        let (cols, lw, lh) = logical_size(cards.len());
        let (pw, ph) = ((lw * scale as f64) as i32, (lh * scale as f64) as i32);
        let attrs = Window::default_attributes()
            .with_title("Frostshot: история")
            .with_decorations(false)
            .with_resizable(false)
            .with_visible(false)
            .with_active(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(PhysicalPosition::new(anchor_x - pw / 2, area.3 - ph))
            .with_inner_size(LogicalSize::new(lw, lh));
        let attrs = crate::platform::overlay_attributes(attrs);
        let window = Rc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        crate::platform::make_no_activate(&window);
        let ctx = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = Surface::new(&ctx, window.clone()).map_err(|e| e.to_string())?;
        let mut p = Self {
            window,
            surface,
            pm: Pixmap::new(1, 1).unwrap(),
            cards,
            cols,
            rects: Vec::new(),
            hover: None,
            cursor: (0.0, 0.0),
            opened: Instant::now(),
            font,
            area,
            anchor_x,
        };
        p.anchor();
        p.present();
        crate::platform::show_no_activate(&p.window);
        Ok(p)
    }

    /// Над значком, над панелью задач, не вылезая за рабочую область.
    pub fn anchor(&self) {
        let size = self.window.outer_size();
        let margin = (8.0 * self.window.scale_factor()) as i32;
        let (w, h) = (size.width as i32, size.height as i32);
        let x = (self.anchor_x - w / 2).clamp(self.area.0 + margin, (self.area.2 - w - margin).max(self.area.0));
        let pos = PhysicalPosition::new(x, self.area.3 - h - margin);
        if self.window.outer_position().ok() != Some(pos) {
            self.window.set_outer_position(pos);
        }
    }

    /// Убрать карточку после удаления снимка и подогнать размер. false: карточек не осталось.
    pub fn remove_card(&mut self, path: &std::path::Path) -> bool {
        self.cards.retain(|c| c.path != path);
        self.hover = None;
        if self.cards.is_empty() {
            return false;
        }
        let (cols, lw, lh) = logical_size(self.cards.len());
        self.cols = cols;
        let _ = self.window.request_inner_size(LogicalSize::new(lw, lh));
        self.anchor();
        self.window.request_redraw();
        true
    }

    /// Идёт анимация появления: нужна перерисовка каждый кадр.
    pub fn animating(&self) -> bool {
        self.opened.elapsed() < ANIM + Duration::from_millis(40 * self.cards.len() as u64)
    }

    pub fn on_move(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
        let hover = self.rects.iter().find(|(_, r)| x >= r.left() && x < r.right() && y >= r.top() && y < r.bottom()).map(|(t, _)| *t);
        if hover != self.hover {
            self.hover = hover;
            self.window.request_redraw();
        }
        self.window.set_cursor(if hover.is_some() { CursorIcon::Pointer } else { CursorIcon::Default });
    }

    pub fn on_click(&self) -> PopupClick {
        match self.hover {
            Some(Target::Card(i)) => self.cards.get(i).map_or(PopupClick::None, |c| PopupClick::Open(c.path.clone())),
            Some(Target::Copy(i)) => self.cards.get(i).map_or(PopupClick::None, |c| PopupClick::Copy(c.path.clone())),
            Some(Target::Delete(i)) => self.cards.get(i).map_or(PopupClick::None, |c| PopupClick::Delete(c.path.clone())),
            Some(Target::Folder) => PopupClick::Folder,
            Some(Target::Clear) => PopupClick::Clear,
            None => PopupClick::None,
        }
    }

    pub fn present(&mut self) {
        let size = self.window.inner_size();
        let (pw, ph) = (size.width.max(1), size.height.max(1));
        if (self.pm.width(), self.pm.height()) != (pw, ph) {
            self.pm = Pixmap::new(pw, ph).unwrap();
        }
        let s = self.window.scale_factor() as f32;
        let t = self.opened.elapsed().as_secs_f32();
        self.rects = draw_panel(&mut self.pm, &self.cards, self.cols, self.hover, t, s, self.font.as_deref());
        let (Some(nw), Some(nh)) = (std::num::NonZeroU32::new(pw), std::num::NonZeroU32::new(ph)) else { return };
        if self.surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = self.surface.buffer_mut() else { return };
        for (dst, px) in buf.iter_mut().zip(self.pm.data().chunks_exact(4)) {
            *dst = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        }
        let _ = buf.present();
    }
}

/// Нарисовать панель в pm; вернуть области кликов. t: секунды с открытия (анимация).
fn draw_panel(pm: &mut Pixmap, cards: &[Card], cols: usize, hover: Option<Target>, t: f32, s: f32, font: Option<&FontVec>) -> Vec<(Target, Rect)> {
    {
        let (w, h) = (pm.width() as f32, pm.height() as f32);
        let mut rects = Vec::new();
        pm.fill(tiny_skia::Color::from_rgba8(BG[0], BG[1], BG[2], 255));
        if let Some(r) = Rect::from_xywh(0.5, 0.5, w - 1.0, h - 1.0) {
            draw::stroke_path(pm, &tiny_skia::PathBuilder::from_rect(r), BORDER, 1.0, 1.0, None);
        }

        let f = crate::ui::ui_font();
        // Заголовок и ссылки.
        if let Some(font) = font {
            let ts = f * s;
            let ty = (HEADER_H * s - draw::line_height(font, ts)) / 2.0;
            draw::draw_text(pm, font, "История", PAD * 1.5 * s, ty, ts, FG, 1.0, None);
            let mut x = w - PAD * 1.5 * s;
            for (target, text) in [(Target::Clear, "Очистить"), (Target::Folder, "Папка")] {
                let ls = (f - 2.0) * s;
                let tw = draw::text_size(font, text, ls).0;
                x -= tw;
                let r = Rect::from_xywh(x - 6.0 * s, 6.0 * s, tw + 12.0 * s, HEADER_H * s - 12.0 * s).unwrap();
                if hover == Some(target) {
                    draw::fill_rounded(pm, r, 5.0 * s, CARD_HOVER, 1.0);
                }
                let ly = (HEADER_H * s - draw::line_height(font, ls)) / 2.0;
                draw::draw_text(pm, font, text, x, ly, ls, if hover == Some(target) { FG } else { ACCENT }, 1.0, None);
                rects.push((target, r));
                x -= 18.0 * s;
            }
        }

        // Карточки: самый свежий снимок в правом нижнем углу, ближе к значку.
        let n = cards.len();
        let rows = n.div_ceil(cols).max(1);
        let (cw, ch) = ((THUMB_W + PAD) * s, (THUMB_H + LABEL_H + PAD) * s);
        for (i, card) in cards.iter().enumerate() {
            let slot = n - 1 - i; // 0 = левый верхний
            let (row, col) = (slot / cols, slot % cols);
            // Появление снизу: нижние ряды первыми, со сдвигом по времени.
            let from_bottom = (rows - 1 - row) as f32;
            let k = ((t - from_bottom * 0.04) / ANIM.as_secs_f32()).clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - k).powi(3);
            let dy = (1.0 - ease) * 60.0 * s;
            if k <= 0.0 {
                continue;
            }
            let x = PAD * s + col as f32 * (cw + PAD * s);
            let y = HEADER_H * s + row as f32 * (ch + PAD * s) + dy;
            let Some(r) = Rect::from_xywh(x, y, cw, ch) else { continue };
            let hovered = hover.and_then(Target::card) == Some(i);
            draw::fill_rounded(pm, r, 8.0 * s, if hovered { CARD_HOVER } else { CARD }, ease);
            if hovered {
                if let Some(path) = draw::rounded_rect(r, 8.0 * s) {
                    draw::stroke_path(pm, &path, ACCENT, 1.0, 1.5 * s, None);
                }
            }
            if let Some(th) = &card.thumb {
                let (bw, bh) = ((THUMB_W - PAD) * s, THUMB_H * s - PAD * s);
                let kk = (bw / th.width() as f32).min(bh / th.height() as f32);
                let (tw, tth) = (th.width() as f32 * kk, th.height() as f32 * kk);
                let (tx, ty) = (x + (cw - tw) / 2.0, y + PAD * s + (bh - tth) / 2.0);
                let paint = PixmapPaint { quality: FilterQuality::Bicubic, opacity: ease, ..PixmapPaint::default() };
                pm.draw_pixmap(0, 0, th.as_ref(), &paint, Transform::from_row(kk, 0.0, 0.0, kk, tx, ty), None);
            }
            if let Some(font) = font {
                let ls = (f - 3.0) * s;
                let tw = draw::text_size(font, &card.label, ls).0;
                draw::draw_text(pm, font, &card.label, x + (cw - tw) / 2.0, y + ch - LABEL_H * s + 2.0 * s, ls, if hovered { FG } else { MUTED }, ease, None);
            }
            if k >= 1.0 {
                // Кнопки на карточке под курсором: копировать и удалить. Их области идут
                // раньше области карточки, чтобы клик по кнопке не открывал снимок.
                if hovered {
                    let bs = 30.0 * s;
                    let by = y + 6.0 * s;
                    let mut bx = x + cw - 6.0 * s - bs;
                    for target in [Target::Delete(i), Target::Copy(i)] {
                        let br = Rect::from_xywh(bx, by, bs, bs).unwrap();
                        let over = hover == Some(target);
                        let fill = match (target, over) {
                            (Target::Delete(_), true) => [0xc0, 0x3a, 0x3a],
                            (_, true) => ACCENT,
                            _ => [0x14, 0x14, 0x17],
                        };
                        draw::fill_rounded(pm, br, bs / 2.0, fill, 0.92);
                        card_icon(pm, target, br, s);
                        rects.push((target, br));
                        bx -= bs + 6.0 * s;
                    }
                }
                rects.push((Target::Card(i), r));
            }
        }
        rects
    }
}

/// Иконки кнопок карточки: две страницы (копировать) и корзина (удалить).
fn card_icon(pm: &mut Pixmap, target: Target, r: Rect, s: f32) {
    let (cx, cy) = (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);
    let w = 1.6 * s;
    let white = [0xff, 0xff, 0xff];
    let outline = |pm: &mut Pixmap, x: f32, y: f32, ww: f32, hh: f32| {
        if let Some(rr) = Rect::from_xywh(x, y, ww, hh) {
            draw::stroke_path(pm, &tiny_skia::PathBuilder::from_rect(rr), white, 1.0, w, None);
        }
    };
    match target {
        Target::Copy(_) => {
            outline(pm, cx - 3.0 * s, cy - 7.0 * s, 9.0 * s, 11.0 * s);
            if let Some(rr) = Rect::from_xywh(cx - 7.0 * s, cy - 4.0 * s, 9.0 * s, 11.0 * s) {
                draw::fill_rect(pm, rr, [0x14, 0x14, 0x17], 1.0, None);
            }
            outline(pm, cx - 7.0 * s, cy - 4.0 * s, 9.0 * s, 11.0 * s);
        }
        Target::Delete(_) => {
            draw::line(pm, cx - 7.0 * s, cy - 5.0 * s, cx + 7.0 * s, cy - 5.0 * s, white, 1.0, w, None);
            draw::line(pm, cx - 2.5 * s, cy - 7.5 * s, cx + 2.5 * s, cy - 7.5 * s, white, 1.0, w, None);
            let mut pb = tiny_skia::PathBuilder::new();
            pb.move_to(cx - 5.5 * s, cy - 5.0 * s);
            pb.line_to(cx - 4.5 * s, cy + 7.5 * s);
            pb.line_to(cx + 4.5 * s, cy + 7.5 * s);
            pb.line_to(cx + 5.5 * s, cy - 5.0 * s);
            if let Some(p) = pb.finish() {
                draw::stroke_path(pm, &p, white, 1.0, w, None);
            }
            draw::line(pm, cx - 1.5 * s, cy - 2.0 * s, cx - 1.5 * s, cy + 5.0 * s, white, 1.0, w * 0.8, None);
            draw::line(pm, cx + 1.5 * s, cy - 2.0 * s, cx + 1.5 * s, cy + 5.0 * s, white, 1.0, w * 0.8, None);
        }
        _ => {}
    }
}

/// Отрисовка панели без окна (для самопроверки): последние снимки, наведение на первый.
/// hover_copy: курсор на кнопке «Копировать» первой карточки.
pub fn render_preview(entries: &[crate::history::Entry], s: f32, t: f32, font: Option<&FontVec>, hover_copy: bool) -> Option<Pixmap> {
    let cards: Vec<Card> = entries
        .iter()
        .map(|e| Card { path: e.path.clone(), label: e.label.clone(), thumb: std::fs::read(&e.thumb).ok().and_then(|b| Pixmap::decode_png(&b).ok()) })
        .collect();
    let (cols, lw, lh) = logical_size(cards.len());
    let mut pm = Pixmap::new((lw as f32 * s) as u32, (lh as f32 * s) as u32)?;
    draw_panel(&mut pm, &cards, cols, Some(if hover_copy { Target::Copy(0) } else { Target::Card(0) }), t, s, font);
    Some(pm)
}
