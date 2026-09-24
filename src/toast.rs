//! Уведомление после снимка: миниатюра, заголовок, клик открывает снимок для доработки.
//! Своё окно без фокуса: одинаково на всех ОС и умеет показывать картинку.
//!
//! Размер задаётся в логических единицах: при создании на мониторе с другим DPI
//! система сама пересчитывает окно, а рисуем по фактическому размеру и масштабу.

use crate::draw::{self, Rgb};
use ab_glyph::FontVec;
use softbuffer::Surface;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Rect, Transform};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event_loop::ActiveEventLoop;
use winit::window::{CursorIcon, Window, WindowLevel};

const BG: Rgb = [0x1f, 0x1f, 0x22];
const BORDER: Rgb = [0x44, 0x44, 0x4b];
const FG: Rgb = [0xe6, 0xe6, 0xe6];
const MUTED: Rgb = [0x9a, 0x9a, 0xa2];
const LIFETIME: Duration = Duration::from_secs(5);

pub enum ToastClick {
    Open,
    Close,
}

pub struct Toast {
    pub window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    pm: Pixmap,
    /// Уменьшенная копия результата с запасом под масштаб до 2x.
    source: Option<Pixmap>,
    title: String,
    subtitle: String,
    font: Option<Arc<FontVec>>,
    shown: Instant,
    hovered: bool,
    hover_close: bool,
    close_rect: Rect,
    /// Рабочая область монитора l, t, r, b: окно прижимается к правому нижнему углу.
    area: (i32, i32, i32, i32),
}

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left() && x < r.right() && y >= r.top() && y < r.bottom()
}

/// Картинка, вписанная в max_w x max_h.
fn fit_image(img: &Pixmap, max_w: f32, max_h: f32) -> Option<Pixmap> {
    let k = (max_w / img.width() as f32).min(max_h / img.height() as f32).min(1.0);
    let (w, h) = ((img.width() as f32 * k).round().max(1.0) as u32, (img.height() as f32 * k).round().max(1.0) as u32);
    let mut t = Pixmap::new(w, h)?;
    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
    t.draw_pixmap(0, 0, img.as_ref(), &paint, Transform::from_scale(k, k), None);
    Some(t)
}

fn logical_size() -> (f64, f64) {
    let f = crate::ui::ui_font() as f64;
    (360.0 + f * 4.0, 96.0 + f)
}

impl Toast {
    /// area: рабочая область монитора (без панели задач), физические пиксели l, t, r, b.
    pub fn open(
        el: &ActiveEventLoop,
        font: Option<Arc<FontVec>>,
        img: &Pixmap,
        title: String,
        subtitle: String,
        area: (i32, i32, i32, i32),
        scale: f32,
    ) -> Result<Self, String> {
        let (lw, lh) = logical_size();
        let margin = (16.0 * scale) as i32;
        let pos = PhysicalPosition::new(
            area.2 - (lw * scale as f64) as i32 - margin,
            area.3 - (lh * scale as f64) as i32 - margin,
        );
        let attrs = Window::default_attributes()
            .with_title("Frostshot")
            .with_decorations(false)
            .with_resizable(false)
            .with_visible(false)
            .with_active(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(pos)
            .with_inner_size(LogicalSize::new(lw, lh));
        let attrs = crate::platform::overlay_attributes(attrs);
        let window = Rc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        crate::platform::make_no_activate(&window);
        let ctx = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = Surface::new(&ctx, window.clone()).map_err(|e| e.to_string())?;
        let f = crate::ui::ui_font();
        let source = fit_image(img, (120.0 + f * 1.5) * 2.0, (96.0 + f) * 2.0);
        let mut t = Self {
            window,
            surface,
            pm: Pixmap::new(1, 1).unwrap(),
            source,
            title,
            subtitle,
            font,
            shown: Instant::now(),
            hovered: false,
            hover_close: false,
            close_rect: Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap(),
            area,
        };
        t.anchor();
        t.present();
        crate::platform::show_no_activate(&t.window);
        Ok(t)
    }

    /// Прижать к правому нижнему углу рабочей области по фактическому размеру.
    pub fn anchor(&self) {
        let size = self.window.outer_size();
        let margin = (16.0 * self.window.scale_factor()) as i32;
        let pos = PhysicalPosition::new(self.area.2 - size.width as i32 - margin, self.area.3 - size.height as i32 - margin);
        if self.window.outer_position().ok() != Some(pos) {
            self.window.set_outer_position(pos);
        }
    }

    /// Когда закрыть; None, пока курсор над уведомлением.
    pub fn deadline(&self) -> Option<Instant> {
        (!self.hovered).then_some(self.shown + LIFETIME)
    }

    pub fn set_hovered(&mut self, on: bool) {
        self.hovered = on;
        if !on {
            // Ушёл курсор: даём ещё полный интервал.
            self.shown = Instant::now();
            self.hover_close = false;
        }
        self.window.request_redraw();
    }

    pub fn on_move(&mut self, x: f32, y: f32) {
        self.hovered = true;
        let hc = contains(&self.close_rect, x, y);
        if hc != self.hover_close {
            self.hover_close = hc;
            self.window.request_redraw();
        }
        self.window.set_cursor(CursorIcon::Pointer);
    }

    pub fn on_click(&self, x: f32, y: f32) -> ToastClick {
        if contains(&self.close_rect, x, y) { ToastClick::Close } else { ToastClick::Open }
    }

    pub fn present(&mut self) {
        let size = self.window.inner_size();
        let (pw, ph) = (size.width.max(1), size.height.max(1));
        if (self.pm.width(), self.pm.height()) != (pw, ph) {
            self.pm = Pixmap::new(pw, ph).unwrap();
        }
        let s = self.window.scale_factor() as f32;
        let (w, h) = (pw as f32, ph as f32);
        let pad = 12.0 * s;
        let thumb = self.source.as_ref().and_then(|src| fit_image(src, w * 0.34, h - 2.0 * pad));
        let pm = &mut self.pm;
        pm.fill(tiny_skia::Color::from_rgba8(BG[0], BG[1], BG[2], 255));
        if let Some(r) = Rect::from_xywh(0.5, 0.5, w - 1.0, h - 1.0) {
            let path = tiny_skia::PathBuilder::from_rect(r);
            draw::stroke_path(pm, &path, BORDER, 1.0, 1.0, None);
        }
        let mut tx = pad;
        if let Some(t) = &thumb {
            let ty = ((h - t.height() as f32) / 2.0).round() as i32;
            pm.draw_pixmap(pad as i32, ty, t.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            tx = pad * 2.0 + t.width() as f32;
        }
        // Крестик.
        let cs = 24.0 * s;
        self.close_rect = Rect::from_xywh(w - cs - 6.0 * s, 6.0 * s, cs, cs).unwrap();
        if self.hover_close {
            draw::fill_rounded(pm, self.close_rect, 4.0 * s, [0x3a, 0x3a, 0x40], 1.0);
        }
        let c = self.close_rect;
        let k = 7.0 * s;
        let (cx, cy) = (c.left() + c.width() / 2.0, c.top() + c.height() / 2.0);
        draw::line(pm, cx - k / 2.0, cy - k / 2.0, cx + k / 2.0, cy + k / 2.0, MUTED, 1.0, 1.5 * s, None);
        draw::line(pm, cx + k / 2.0, cy - k / 2.0, cx - k / 2.0, cy + k / 2.0, MUTED, 1.0, 1.5 * s, None);

        if let Some(font) = self.font.as_deref() {
            let f = crate::ui::ui_font();
            let ts = f * s;
            let ss = (f - 3.0) * s;
            let lh_t = draw::line_height(font, ts);
            let lh_s = draw::line_height(font, ss);
            let block = lh_t + 4.0 * s + lh_s;
            let y0 = (h - block) / 2.0;
            let avail = c.left() - tx - 4.0 * s;
            let title = fit(font, &self.title, ts, avail);
            let subtitle = fit(font, &self.subtitle, ss, w - tx - pad);
            draw::draw_text(pm, font, &title, tx, y0, ts, FG, 1.0, None);
            draw::draw_text(pm, font, &subtitle, tx, y0 + lh_t + 4.0 * s, ss, MUTED, 1.0, None);
        }

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

/// Обрезать строку с многоточием под ширину.
fn fit(font: &FontVec, text: &str, size: f32, avail: f32) -> String {
    if draw::text_size(font, text, size).0 <= avail {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let t: String = chars.iter().collect::<String>() + "…";
        if draw::text_size(font, &t, size).0 <= avail {
            return t;
        }
    }
    "…".into()
}
