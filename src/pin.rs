//! Снимок, закреплённый поверх всех окон. Тащится мышью, колесо масштабирует,
//! Ctrl+C копирует, двойной клик или Esc закрывает.
//!
//! Размер задаётся в логических единицах от масштаба целевого монитора: после
//! пересчёта DPI окно получает ровно физический размер снимка (пиксель в пиксель).

use softbuffer::Surface;
use std::rc::Rc;
use std::time::{Duration, Instant};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowLevel};

const BG: [u8; 3] = [0x2a, 0x2a, 0x2f];
const BORDER: [u8; 3] = crate::ui::ACCENT;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub enum PinAction {
    None,
    Close,
    Copy,
}

pub struct Pin {
    pub window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    pub img: Pixmap,
    frame: Pixmap,
    zoom: f32,
    ctrl: bool,
    last_click: Option<Instant>,
}

impl Pin {
    /// x, y: левый верхний угол в глобальных физических координатах; scale: масштаб монитора.
    pub fn open(el: &ActiveEventLoop, img: Pixmap, x: i32, y: i32, scale: f32) -> Result<Self, String> {
        let s = scale.max(0.1) as f64;
        let attrs = Window::default_attributes()
            .with_title("Frostshot: закреплённый снимок")
            .with_decorations(false)
            .with_resizable(false)
            .with_visible(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(PhysicalPosition::new(x, y))
            .with_inner_size(LogicalSize::new(img.width() as f64 / s, img.height() as f64 / s));
        let attrs = crate::platform::overlay_attributes(attrs);
        let window = Rc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let ctx = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = Surface::new(&ctx, window.clone()).map_err(|e| e.to_string())?;
        let mut p = Self { window, surface, img, frame: Pixmap::new(1, 1).unwrap(), zoom: 1.0, ctrl: false, last_click: None };
        p.present();
        p.window.set_visible(true);
        p.window.set_outer_position(PhysicalPosition::new(x, y));
        p.window.set_cursor(CursorIcon::Move);
        crate::platform::force_foreground(&p.window);
        Ok(p)
    }

    fn present(&mut self) {
        let size = self.window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        if (self.frame.width(), self.frame.height()) != (w, h) {
            self.frame = Pixmap::new(w, h).unwrap();
        }
        self.frame.fill(tiny_skia::Color::from_rgba8(BG[0], BG[1], BG[2], 255));
        let (kx, ky) = (w as f32 / self.img.width() as f32, h as f32 / self.img.height() as f32);
        let exact = (kx - 1.0).abs() < 1e-3 && (ky - 1.0).abs() < 1e-3;
        let quality = if exact { FilterQuality::Nearest } else { FilterQuality::Bicubic };
        let paint = PixmapPaint { quality, ..PixmapPaint::default() };
        self.frame.draw_pixmap(0, 0, self.img.as_ref(), &paint, Transform::from_scale(kx, ky), None);
        // Рамка, чтобы закреплённый снимок отличался от живого окна под ним.
        let (fw, fh) = (w as f32, h as f32);
        for (x0, y0, x1, y1) in [(0.0, 0.5, fw, 0.5), (0.0, fh - 0.5, fw, fh - 0.5), (0.5, 0.0, 0.5, fh), (fw - 0.5, 0.0, fw - 0.5, fh)] {
            crate::draw::line(&mut self.frame, x0, y0, x1, y1, BORDER, 1.0, 1.0, None);
        }
        let (Some(nw), Some(nh)) = (std::num::NonZeroU32::new(w), std::num::NonZeroU32::new(h)) else { return };
        if self.surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = self.surface.buffer_mut() else { return };
        for (dst, px) in buf.iter_mut().zip(self.frame.data().chunks_exact(4)) {
            *dst = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        }
        let _ = buf.present();
    }

    pub fn on_event(&mut self, event: WindowEvent) -> PinAction {
        match event {
            WindowEvent::RedrawRequested => self.present(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => self.window.request_redraw(),
            WindowEvent::CloseRequested => return PinAction::Close,
            WindowEvent::ModifiersChanged(m) => self.ctrl = m.state().control_key(),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let now = Instant::now();
                if self.last_click.is_some_and(|t| now - t <= DOUBLE_CLICK) {
                    return PinAction::Close;
                }
                self.last_click = Some(now);
                let _ = self.window.drag_window();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                if d != 0.0 {
                    self.zoom = (self.zoom * if d > 0.0 { 1.1 } else { 1.0 / 1.1 }).clamp(0.1, 5.0);
                    if (self.zoom - 1.0).abs() < 0.05 {
                        self.zoom = 1.0;
                    }
                    let (w, h) = ((self.img.width() as f32 * self.zoom).round().max(8.0), (self.img.height() as f32 * self.zoom).round().max(8.0));
                    let _ = self.window.request_inner_size(PhysicalSize::new(w as u32, h as u32));
                    self.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => match event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => return PinAction::Close,
                PhysicalKey::Code(KeyCode::KeyC) if self.ctrl => return PinAction::Copy,
                _ => {}
            },
            _ => {}
        }
        PinAction::None
    }
}
