#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_overlay;
mod app_settings;
mod capture;
mod config;
mod draw;
mod output;
mod pin;
mod platform;
mod project;
mod selection;
mod selftest;
mod session;
mod settings;
mod shapes;
mod toast;
mod tray;
mod ui;
mod upload;

use ab_glyph::FontVec;
use app_overlay::Pending;
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
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId, WindowLevel};

/// Сколько хранить последний снимок для повторного открытия.
const LAST_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

enum UserEvent {
    Hotkey(GlobalHotKeyEvent),
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    SaveChosen(Option<PathBuf>),
    DirChosen(Option<PathBuf>),
    ProjectChosen(Option<PathBuf>),
    /// Аргументы от второго экземпляра (PrintScreen через Windows, двойной клик по .frost).
    Remote(Vec<String>),
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
    /// Открыт из уведомления, трея или проекта: при Esc сессия остаётся последней.
    reopened: bool,
}

struct App {
    config: Config,
    proxy: EventLoopProxy<UserEvent>,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey_ids: Vec<u32>,
    registered: Vec<HotKey>,
    settings: Option<settings::Settings>,
    tray: Option<tray::Tray>,
    overlay: Option<Overlay>,
    /// Последний снимок (усыплённая сессия) для повторного открытия.
    last: Option<Session>,
    /// Когда last перестал использоваться: через LAST_TTL память освобождается.
    last_at: std::time::Instant,
    toast: Option<toast::Toast>,
    /// Снимки, закреплённые поверх окон.
    pins: Vec<pin::Pin>,
    toast_pos: (f32, f32),
    pending_save: Option<Pending>,
    clipboard: Option<arboard::Clipboard>,
    font: Option<Arc<FontVec>>,
    started: bool,
}

impl App {
    fn init(&mut self) {
        match GlobalHotKeyManager::new() {
            Ok(mgr) => self.hotkeys = Some(mgr),
            Err(e) => log::error!("hotkey manager: {e}"),
        }
        let (label, _) = self.register_hotkeys();

        // Автозапуск: реальное состояние главнее конфига; путь обновляем на текущий exe.
        self.config.autostart = platform::autostart_enabled();
        if self.config.autostart {
            if let Err(e) = platform::set_autostart(true) {
                log::warn!("autostart refresh: {e}");
            }
        }

        match tray::Tray::build(&label, self.config.autostart) {
            Ok(t) => self.tray = Some(t),
            Err(e) => log::error!("tray: {e}"),
        }
        self.clipboard = arboard::Clipboard::new().map_err(|e| log::warn!("clipboard: {e}")).ok();

        // Регистрация в Windows указывает на путь exe: после переноса программы обновляем её.
        if platform::shell_status().registered {
            if let Err(e) = platform::shell_register(true) {
                log::warn!("shell integration refresh: {e}");
            }
        }

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

    /// Команды из командной строки или от второго экземпляра.
    fn handle_args(&mut self, el: &ActiveEventLoop, args: &[String]) {
        log::info!("args: {args:?}");
        if args.iter().any(|a| a == "--settings") {
            self.open_settings(el);
        }
        if let Some(p) = args.iter().find(|a| a.to_lowercase().ends_with(".frost")) {
            self.open_project(el, std::path::Path::new(p));
        } else if args.iter().any(|a| a == "--capture" || a.to_lowercase().starts_with("ms-screenclip:")) {
            self.start_capture(el);
        }
    }

    fn handle_window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.settings.as_ref().is_some_and(|s| s.window.id() == id) {
            self.handle_settings_event(event);
        } else if self.toast.as_ref().is_some_and(|t| t.window.id() == id) {
            self.handle_toast_event(el, event);
        } else if let Some(i) = self.pins.iter().position(|p| p.window.id() == id) {
            match self.pins[i].on_event(event) {
                pin::PinAction::Close => {
                    self.pins.remove(i);
                }
                pin::PinAction::Copy => {
                    if self.clipboard.is_none() {
                        self.clipboard = arboard::Clipboard::new().ok();
                    }
                    if let Some(cb) = self.clipboard.as_mut() {
                        if let Err(e) = output::to_clipboard(cb, &self.pins[i].img) {
                            log::error!("clipboard: {e}");
                        }
                    }
                }
                pin::PinAction::None => {}
            }
        } else {
            self.handle_overlay_event(el, id, event);
        }
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
            self.toast = None;
        }
        self.request_redraws();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if !self.started {
            self.started = true;
            let args: Vec<String> = std::env::args().skip(1).collect();
            self.guarded(|app| {
                app.init();
                app.handle_args(el, &args);
            });
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
                } else if e.id == t.last_id {
                    app.reopen_last(el);
                } else if e.id == t.open_project_id {
                    app.pick_project();
                } else if e.id == t.folder_id {
                    platform::open_folder(&app.config.save_dir);
                } else if e.id == t.settings_id {
                    app.open_settings(el);
                } else if e.id == t.autostart_id {
                    let on = !app.config.autostart;
                    app.set_autostart(on);
                    if let Some(st) = &app.settings {
                        st.window.request_redraw();
                    }
                } else if e.id == t.quit_id {
                    app.close_overlay();
                    el.exit();
                }
            }
            UserEvent::SaveChosen(p) => app.on_save_chosen(el, p),
            UserEvent::DirChosen(p) => {
                if let Some(p) = p {
                    app.config.save_dir = p;
                    app.config.save();
                }
                if let Some(st) = &app.settings {
                    st.window.request_redraw();
                }
            }
            UserEvent::Remote(args) => app.handle_args(el, &args),
            UserEvent::ProjectChosen(p) => {
                if let Some(p) = p {
                    app.open_project(el, &p);
                }
            }
        });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.guarded(|app| app.handle_window_event(el, id, event));
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = std::time::Instant::now();
        // Таймер уведомления.
        if self.toast.as_ref().and_then(|t| t.deadline()).is_some_and(|d| now >= d) {
            self.toast = None;
        }
        // Последний снимок держит полные кадры мониторов: освобождаем после простоя.
        if self.last.is_some() && self.toast.is_none() && now >= self.last_at + LAST_TTL {
            self.last = None;
            if let Some(t) = &self.tray {
                t.set_last_enabled(false);
            }
            log::info!("last shot released after {} min", LAST_TTL.as_secs() / 60);
        }
        let toast_d = self.toast.as_ref().and_then(|t| t.deadline());
        let last_d = self.last.as_ref().map(|_| self.last_at + LAST_TTL);
        match toast_d.into_iter().chain(last_d).min() {
            Some(d) => el.set_control_flow(ControlFlow::WaitUntil(d)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }
}

/// Лог: в release в файл (с ротацией на 1 МБ), в debug в stderr. Паники тоже в лог.
fn init_logging() {
    let mut b = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if !cfg!(debug_assertions) {
        if let Some(path) = platform::log_path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024) {
                let _ = std::fs::rename(&path, path.with_extension("old.log"));
            }
            if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                b.target(env_logger::Target::Pipe(Box::new(f)));
            }
        }
    }
    b.init();
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture());
    }));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--selftest") {
        let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("selftest-out"));
        std::process::exit(selftest::run(&dir));
    }
    if args.get(1).map(String::as_str) == Some("--write-icon") {
        let path = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("assets/frostshot.ico"));
        match tray::write_ico(&path) {
            Ok(()) => println!("written {}", path.display()),
            Err(e) => eprintln!("write icon: {e}"),
        }
        return;
    }
    init_logging();

    // Debug-сборка не конфликтует с установленной release-версией.
    let instance_name = if cfg!(debug_assertions) { "frostshot-dev-instance-7c1e" } else { "frostshot-single-instance-7c1e" };
    let instance = single_instance::SingleInstance::new(instance_name).ok();
    if instance.as_ref().is_some_and(|i| !i.is_single()) {
        // Уже запущен: передаём ему команду (пустой запуск открывает настройки).
        let mut fwd: Vec<String> = args[1..].to_vec();
        if fwd.is_empty() {
            fwd.push("--settings".into());
        }
        let sent = platform::ipc_send(&fwd);
        log::info!("already running, forwarded {fwd:?}: {sent}");
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

    {
        let p = proxy.clone();
        platform::ipc_listen(move |args| {
            let _ = p.send_event(UserEvent::Remote(args));
        });
    }

    let config = Config::load();
    ui::set_font_size(config.ui_font_size);
    let mut app = App {
        config,
        proxy,
        hotkeys: None,
        hotkey_ids: Vec::new(),
        registered: Vec::new(),
        settings: None,
        tray: None,
        overlay: None,
        last: None,
        last_at: std::time::Instant::now(),
        toast: None,
        pins: Vec::new(),
        toast_pos: (0.0, 0.0),
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
