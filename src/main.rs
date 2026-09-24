#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod config;
mod draw;
mod output;
mod platform;
mod selection;
mod selftest;
mod session;
mod shapes;
mod tray;
mod ui;

use ab_glyph::FontVec;
use config::Config;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use session::{Action, Session};
use softbuffer::Surface;
use std::num::NonZeroU32;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use tiny_skia::Pixmap;
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton as TrayButton, MouseButtonState, TrayIconEvent};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId, WindowLevel};

enum UserEvent {
    Hotkey(GlobalHotKeyEvent),
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    SaveChosen(Option<PathBuf>),
}

struct OverlayWin {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    mon: usize,
    pos: (f32, f32),
}

struct Overlay {
    wins: Vec<OverlayWin>,
    session: Session,
}

struct App {
    config: Config,
    proxy: EventLoopProxy<UserEvent>,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey_ids: Vec<u32>,
    tray: Option<tray::Tray>,
    overlay: Option<Overlay>,
    pending_save: Option<Pixmap>,
    clipboard: Option<arboard::Clipboard>,
    font: Option<Arc<FontVec>>,
    started: bool,
}

impl App {
    fn init(&mut self) {
        // Хоткеи: основной и запасной.
        let mut label = String::new();
        match GlobalHotKeyManager::new() {
            Ok(mgr) => {
                for s in [self.config.hotkey.clone(), self.config.fallback_hotkey.clone()] {
                    match s.parse::<HotKey>() {
                        Ok(hk) => match mgr.register(hk) {
                            Ok(()) => {
                                self.hotkey_ids.push(hk.id());
                                if label.is_empty() {
                                    label = s.clone();
                                }
                                log::info!("hotkey registered: {s}");
                            }
                            Err(e) => log::warn!("hotkey {s} not registered: {e}"),
                        },
                        Err(e) => log::warn!("hotkey {s} parse error: {e}"),
                    }
                }
                self.hotkeys = Some(mgr);
            }
            Err(e) => log::error!("hotkey manager: {e}"),
        }
        if label.is_empty() {
            label = "клик по иконке".into();
        }

        match tray::build(&label) {
            Ok(t) => self.tray = Some(t),
            Err(e) => log::error!("tray: {e}"),
        }
        self.clipboard = arboard::Clipboard::new().map_err(|e| log::warn!("clipboard: {e}")).ok();

        if self.config.hotkey.eq_ignore_ascii_case("PrintScreen")
            && platform::printscreen_taken_by_system()
            && !self.config.printscreen_warned
        {
            self.config.printscreen_warned = true;
            self.config.save();
            let fallback = self.config.fallback_hotkey.clone();
            std::thread::spawn(move || {
                rfd::MessageDialog::new()
                    .set_title("Frostshot")
                    .set_description(format!(
                        "Клавишу PrintScreen сейчас перехватывает Windows (Snipping Tool).\n\n\
                         Чтобы Frostshot работал по PrintScreen, выключите:\n\
                         Параметры > Специальные возможности > Клавиатура >\n\
                         «Использовать клавишу PrtScn для открытия Ножниц».\n\n\
                         Пока можно использовать {fallback} или клик по иконке в трее."
                    ))
                    .set_level(rfd::MessageLevel::Info)
                    .show();
            });
        }
    }

    fn start_capture(&mut self, el: &ActiveEventLoop) {
        if self.overlay.is_some() || self.pending_save.is_some() {
            return;
        }
        let t0 = std::time::Instant::now();
        let shots = match capture::capture_all() {
            Ok(s) => s,
            Err(e) => {
                log::error!("capture failed: {e}");
                return;
            }
        };
        log::info!("captured {} monitors in {:?}", shots.len(), t0.elapsed());

        let monitors: Vec<_> = el.available_monitors().collect();
        let mut wins = Vec::new();
        let mut scales = Vec::new();
        for (i, shot) in shots.iter().enumerate() {
            let handle = match_monitor(&monitors, shot);
            let mut attrs = Window::default_attributes()
                .with_title("Frostshot")
                .with_decorations(false)
                .with_resizable(false)
                .with_visible(false)
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_position(PhysicalPosition::new(shot.x, shot.y))
                .with_inner_size(PhysicalSize::new(shot.width(), shot.height()));
            if let Some(h) = &handle {
                attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(Some(h.clone()))));
            }
            attrs = platform::overlay_attributes(attrs);
            let window = match el.create_window(attrs) {
                Ok(w) => Rc::new(w),
                Err(e) => {
                    log::error!("create window: {e}");
                    return;
                }
            };
            let surface = match softbuffer::Context::new(window.clone()).and_then(|c| Surface::new(&c, window.clone())) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("softbuffer: {e}");
                    return;
                }
            };
            let scale = handle.as_ref().map(|h| h.scale_factor()).unwrap_or(1.0) as f32;
            log::info!("monitor {i}: {},{} {}x{} scale {scale}", shot.x, shot.y, shot.width(), shot.height());
            scales.push(scale);
            wins.push(OverlayWin { window, surface, mon: i, pos: (0.0, 0.0) });
        }

        let cursor = platform::cursor_pos().and_then(|(gx, gy)| {
            shots
                .iter()
                .position(|s| s.contains_global(gx, gy))
                .map(|i| (i, (gx - shots[i].x) as f32, (gy - shots[i].y) as f32))
        });
        let mut session = Session::new(shots, scales, self.config.color, self.config.width, self.font.clone());
        if let Some((m, x, y)) = cursor {
            session.on_move(m, x, y);
            if let Some(w) = wins.iter_mut().find(|w| w.mon == m) {
                w.pos = (x, y);
            }
        }
        let mut ov = Overlay { wins, session };
        for w in &mut ov.wins {
            present(w, &mut ov.session);
            w.window.set_visible(true);
            w.window.set_cursor(ov.session.cursor_icon(w.mon));
        }
        let focus = cursor.map(|c| c.0).unwrap_or(0);
        if let Some(w) = ov.wins.iter().find(|w| w.mon == focus) {
            w.window.focus_window();
        }
        log::info!("overlay shown in {:?}", t0.elapsed());
        self.overlay = Some(ov);
    }

    fn close_overlay(&mut self) {
        if let Some(ov) = self.overlay.take() {
            if ov.session.color != self.config.color || ov.session.width != self.config.width {
                self.config.color = ov.session.color;
                self.config.width = ov.session.width;
                self.config.save();
            }
        }
        self.pending_save = None;
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Close => self.close_overlay(),
            Action::Copy => {
                let Some(img) = self.overlay.as_mut().and_then(|o| o.session.result()) else { return };
                if self.clipboard.is_none() {
                    self.clipboard = arboard::Clipboard::new().ok();
                }
                match self.clipboard.as_mut().map(|cb| output::to_clipboard(cb, &img)) {
                    Some(Ok(())) => log::info!("copied {}x{}", img.width(), img.height()),
                    Some(Err(e)) => log::error!("clipboard: {e}"),
                    None => log::error!("clipboard unavailable"),
                }
                self.close_overlay();
            }
            Action::QuickSave => {
                let Some(img) = self.overlay.as_mut().and_then(|o| o.session.result()) else { return };
                let path = self.config.save_dir.join(output::default_file_name());
                match output::save_png(&img, &path) {
                    Ok(p) => log::info!("saved {}", p.display()),
                    Err(e) => log::error!("save: {e}"),
                }
                self.close_overlay();
            }
            Action::Save => {
                let Some(img) = self.overlay.as_mut().and_then(|o| o.session.result()) else { return };
                // Оверлей поверх всех окон перекрыл бы диалог: прячем на время выбора файла.
                if let Some(ov) = &self.overlay {
                    for w in &ov.wins {
                        w.window.set_visible(false);
                    }
                }
                self.pending_save = Some(img);
                let dir = self.config.save_dir.clone();
                let _ = std::fs::create_dir_all(&dir);
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let path = rfd::FileDialog::new()
                        .set_title("Сохранить скриншот")
                        .set_directory(&dir)
                        .set_file_name(output::default_file_name())
                        .add_filter("PNG", &["png"])
                        .save_file();
                    let _ = proxy.send_event(UserEvent::SaveChosen(path));
                });
            }
        }
    }

    fn on_save_chosen(&mut self, path: Option<PathBuf>) {
        let Some(img) = self.pending_save.take() else { return };
        match path {
            Some(path) => {
                match output::save_png(&img, &path) {
                    Ok(p) => {
                        log::info!("saved {}", p.display());
                        if let Some(dir) = p.parent() {
                            self.config.save_dir = dir.to_path_buf();
                            self.config.save();
                        }
                    }
                    Err(e) => log::error!("save: {e}"),
                }
                self.close_overlay();
            }
            None => {
                // Отмена: возвращаем оверлей как был.
                if let Some(ov) = &mut self.overlay {
                    for w in &mut ov.wins {
                        w.window.set_visible(true);
                        present(w, &mut ov.session);
                    }
                    if let Some(w) = ov.wins.first() {
                        w.window.focus_window();
                    }
                }
            }
        }
    }

    fn handle_window_event(&mut self, id: WindowId, event: WindowEvent) {
        let Some(ov) = &mut self.overlay else { return };
        let Some(idx) = ov.wins.iter().position(|w| w.window.id() == id) else { return };
        let mon = ov.wins[idx].mon;
        let s = &mut ov.session;
        let mut action = Action::None;
        match event {
            WindowEvent::RedrawRequested => {
                present(&mut ov.wins[idx], s);
                return;
            }
            WindowEvent::CloseRequested => action = Action::Close,
            WindowEvent::CursorMoved { position, .. } => {
                let p = (position.x as f32, position.y as f32);
                ov.wins[idx].pos = p;
                s.on_move(mon, p.0, p.1);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (x, y) = ov.wins[idx].pos;
                action = match (button, state) {
                    (MouseButton::Left, ElementState::Pressed) => s.on_left_press(mon, x, y),
                    (MouseButton::Left, ElementState::Released) => s.on_left_release(mon, x, y),
                    (MouseButton::Right, ElementState::Pressed) => s.on_right_press(),
                    _ => Action::None,
                };
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                if d != 0.0 {
                    s.on_wheel(d);
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                let st = m.state();
                s.mods = session::Mods { shift: st.shift_key(), ctrl: st.control_key(), alt: st.alt_key() };
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let code = match event.physical_key {
                    PhysicalKey::Code(c) => Some(c),
                    _ => None,
                };
                let named = match &event.logical_key {
                    Key::Named(n) => Some(*n),
                    _ => None,
                };
                action = s.on_key(code, named, event.text.as_deref());
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                s.dirty[mon] = true;
            }
            _ => {}
        }
        ov.wins[idx].window.set_cursor(s.cursor_icon(mon));
        self.handle_action(action);
    }

    fn request_redraws(&self) {
        if let Some(ov) = &self.overlay {
            for w in &ov.wins {
                if ov.session.dirty[w.mon] {
                    w.window.request_redraw();
                }
            }
        }
    }

    /// Паника в обработчике не валит процесс: закрываем оверлей, остаёмся в трее (инвариант 7).
    fn guarded(&mut self, f: impl FnOnce(&mut Self)) {
        if std::panic::catch_unwind(AssertUnwindSafe(|| f(self))).is_err() {
            log::error!("panic in handler, overlay closed");
            self.overlay = None;
            self.pending_save = None;
        }
        self.request_redraws();
    }
}

/// Монитор winit для снимка xcap: точное совпадение позиции, иначе наибольшее пересечение.
fn match_monitor(monitors: &[winit::monitor::MonitorHandle], shot: &capture::MonitorShot) -> Option<winit::monitor::MonitorHandle> {
    if let Some(m) = monitors.iter().find(|m| m.position() == PhysicalPosition::new(shot.x, shot.y)) {
        return Some(m.clone());
    }
    let (sx0, sy0) = (shot.x as i64, shot.y as i64);
    let (sx1, sy1) = (sx0 + shot.width() as i64, sy0 + shot.height() as i64);
    let best = monitors
        .iter()
        .map(|m| {
            let (p, sz) = (m.position(), m.size());
            let ix = (sx1.min(p.x as i64 + sz.width as i64) - sx0.max(p.x as i64)).max(0);
            let iy = (sy1.min(p.y as i64 + sz.height as i64) - sy0.max(p.y as i64)).max(0);
            (ix * iy, m)
        })
        .filter(|(area, _)| *area > 0)
        .max_by_key(|(area, _)| *area)
        .map(|(_, m)| m.clone());
    log::warn!(
        "monitor at {},{} {}x{}: no exact winit match, fallback {:?}",
        shot.x,
        shot.y,
        shot.width(),
        shot.height(),
        best.as_ref().map(|m| (m.position(), m.size()))
    );
    best
}

fn present(w: &mut OverlayWin, s: &mut Session) {
    let size = w.window.inner_size();
    let (Some(bw), Some(bh)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
    if let Err(e) = w.surface.resize(bw, bh) {
        log::error!("surface resize: {e}");
        return;
    }
    let frame = s.render(w.mon);
    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    let (bw, bh) = (size.width as usize, size.height as usize);
    let data = frame.data();
    let mut buf = match w.surface.buffer_mut() {
        Ok(b) => b,
        Err(e) => {
            log::error!("surface buffer: {e}");
            return;
        }
    };
    let cw = fw.min(bw);
    for y in 0..bh {
        let row = &mut buf[y * bw..(y + 1) * bw];
        if y >= fh {
            row.fill(0);
            continue;
        }
        let src = &data[y * fw * 4..(y * fw + cw) * 4];
        for (dst, px) in row[..cw].iter_mut().zip(src.chunks_exact(4)) {
            *dst = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        }
        row[cw..].fill(0);
    }
    if let Err(e) = buf.present() {
        log::error!("present: {e}");
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, _el: &ActiveEventLoop) {
        if !self.started {
            self.started = true;
            self.guarded(|app| app.init());
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        self.guarded(|app| match event {
            UserEvent::Hotkey(e) => {
                if e.state == HotKeyState::Pressed && app.hotkey_ids.contains(&e.id) {
                    app.start_capture(el);
                }
            }
            UserEvent::Tray(TrayIconEvent::Click { button: TrayButton::Left, button_state: MouseButtonState::Up, .. }) => {
                app.start_capture(el);
            }
            UserEvent::Tray(_) => {}
            UserEvent::Menu(e) => {
                let Some(t) = &app.tray else { return };
                if e.id == t.capture_id {
                    app.start_capture(el);
                } else if e.id == t.folder_id {
                    platform::open_folder(&app.config.save_dir);
                } else if e.id == t.quit_id {
                    app.close_overlay();
                    el.exit();
                }
            }
            UserEvent::SaveChosen(p) => app.on_save_chosen(p),
        });
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.guarded(|app| app.handle_window_event(id, event));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--selftest") {
        let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("selftest-out"));
        std::process::exit(selftest::run(&dir));
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let instance = single_instance::SingleInstance::new("frostshot-single-instance-7c1e").ok();
    if instance.as_ref().is_some_and(|i| !i.is_single()) {
        log::info!("already running");
        return;
    }

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            log::error!("event loop: {e}");
            return;
        }
    };
    let proxy = event_loop.create_proxy();
    {
        let p = proxy.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Hotkey(e));
        }));
        let p = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Tray(e));
        }));
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Menu(e));
        }));
    }

    let mut app = App {
        config: Config::load(),
        proxy,
        hotkeys: None,
        hotkey_ids: Vec::new(),
        tray: None,
        overlay: None,
        pending_save: None,
        clipboard: None,
        font: draw::load_font().map(Arc::new),
        started: false,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("event loop: {e}");
    }
    drop(instance);
}
