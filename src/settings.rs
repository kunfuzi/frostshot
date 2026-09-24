//! Окно настроек: одна страница, рисуется tiny-skia, изменения применяются сразу.

use crate::config::{self, Config};
use crate::draw::{self, Rgb};
use crate::session::Mods;
use ab_glyph::FontVec;
use global_hotkey::hotkey::HotKey;
use softbuffer::Surface;
use std::rc::Rc;
use std::sync::Arc;
use tiny_skia::{Pixmap, Rect};
use winit::dpi::LogicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, NamedKey};
use winit::window::{CursorIcon, Window, WindowButtons};

const BG: Rgb = [0x1b, 0x1b, 0x1e];
const FIELD: Rgb = [0x2a, 0x2a, 0x2f];
const FIELD_HOVER: Rgb = [0x33, 0x33, 0x39];
const BORDER: Rgb = [0x44, 0x44, 0x4b];
const FG: Rgb = [0xe6, 0xe6, 0xe6];
const MUTED: Rgb = [0x9a, 0x9a, 0xa2];
const ERR: Rgb = [0xf0, 0x6a, 0x6a];
const ACCENT: Rgb = crate::ui::ACCENT;
const WIDTH: f64 = 700.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ctl {
    Autostart,
    Notify,
    SaveOnCopy,
    DirChange,
    DirOpen,
    Template,
    Hotkey(usize),
    Dim,
    Font,
    OpenConfig,
    Reset,
    Close,
    ShellRegister,
    ShellUnregister,
    DefaultApps,
    KeyboardSettings,
}

/// Побочные эффекты, которые применяет App.
#[derive(Default, Debug)]
pub struct Fx {
    pub save: bool,
    pub autostart: Option<bool>,
    /// Some(true): начать запись хоткея (снять регистрацию), Some(false): закончить.
    pub record: Option<bool>,
    pub hotkeys: bool,
    pub font: bool,
    pub choose_dir: bool,
    pub open_dir: bool,
    pub open_config: bool,
    pub reset: bool,
    pub close: bool,
    /// Записать (true) или убрать (false) Frostshot из обработчиков PrintScreen Windows.
    pub shell_register: Option<bool>,
    pub default_apps: bool,
    pub keyboard_settings: bool,
}

pub struct Settings {
    pub window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    pm: Pixmap,
    rects: Vec<(Ctl, Rect)>,
    hover: Option<Ctl>,
    slider: Option<Ctl>,
    focus_template: bool,
    template: String,
    pub recording: Option<usize>,
    pub hotkey_err: [Option<String>; 2],
    record_msg: Option<String>,
    pub mods: Mods,
    cursor: (f32, f32),
    font: Option<Arc<FontVec>>,
    sized: bool,
    pub shell: crate::platform::ShellStatus,
}

const KEYS: [(&str, &str); 21] = [
    ("V / L", "рамка / лассо"),
    ("1-9, 0", "инструменты"),
    ("Shift + протягивание", "добавить область"),
    ("Alt + протягивание", "вырезать область"),
    ("Колесо мыши", "толщина"),
    ("Ctrl + колесо", "масштаб, Ctrl+0: 100%"),
    ("Средняя кнопка", "сдвиг при масштабе"),
    ("Shift при рисовании", "45° / квадрат"),
    ("Ctrl+Z", "отменить"),
    ("Ctrl+Shift+Z, Ctrl+Y", "повторить"),
    ("P", "закрепить поверх окон"),
    ("H", "скрыть личные данные"),
    ("Ctrl+Shift+C", "копировать текст"),
    ("R", "линейка"),
    ("Счётчик: тянуть", "от номера к цели"),
    ("Shift + счётчик", "от цели к номеру"),
    ("Ctrl+C, Enter", "копировать"),
    ("Ctrl+S", "сохранить как"),
    ("Ctrl+Shift+S", "сохранить сразу"),
    ("Правый клик", "сбросить выделение"),
    ("Esc", "закрыть"),
];

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left() && x < r.right() && y >= r.top() && y < r.bottom()
}

fn key_name(code: KeyCode) -> Option<String> {
    use KeyCode::*;
    if matches!(
        code,
        ShiftLeft | ShiftRight | ControlLeft | ControlRight | AltLeft | AltRight | SuperLeft | SuperRight | Meta | Hyper | Fn | FnLock
    ) {
        return None;
    }
    let d = format!("{code:?}");
    let name = d
        .strip_prefix("Key")
        .or_else(|| d.strip_prefix("Digit"))
        .map(str::to_string)
        .unwrap_or(d.clone());
    Some(name)
}

impl Settings {
    pub fn open(el: &ActiveEventLoop, font: Option<Arc<FontVec>>, cfg: &Config) -> Result<Self, String> {
        let icon = crate::tray::icon_pixmap(32);
        let icon = winit::window::Icon::from_rgba(crate::tray::rgba_straight(&icon), 32, 32).ok();
        let attrs = Window::default_attributes()
            .with_title("Frostshot: настройки")
            .with_inner_size(LogicalSize::new(WIDTH, 760.0))
            .with_resizable(false)
            .with_enabled_buttons(WindowButtons::CLOSE | WindowButtons::MINIMIZE)
            .with_visible(false)
            .with_window_icon(icon);
        let window = Rc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let ctx = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = Surface::new(&ctx, window.clone()).map_err(|e| e.to_string())?;
        let mut s = Self {
            window,
            surface,
            pm: Pixmap::new(1, 1).unwrap(),
            rects: Vec::new(),
            hover: None,
            slider: None,
            focus_template: false,
            template: cfg.file_template.clone(),
            recording: None,
            hotkey_err: [None, None],
            record_msg: None,
            mods: Mods::default(),
            cursor: (0.0, 0.0),
            font,
            sized: false,
            shell: crate::platform::shell_status(),
        };
        s.present(cfg);
        s.window.set_visible(true);
        s.window.focus_window();
        Ok(s)
    }

    /// Перечитать состояние интеграции с Windows (пользователь мог поменять его в параметрах).
    pub fn refresh_shell(&mut self) {
        self.shell = crate::platform::shell_status();
        self.window.request_redraw();
    }

    pub fn reload(&mut self, cfg: &Config) {
        self.template = cfg.file_template.clone();
        self.hotkey_err = [None, None];
    }

    fn hit(&self, x: f32, y: f32) -> Option<Ctl> {
        self.rects.iter().find(|(_, r)| contains(r, x, y)).map(|(c, _)| *c)
    }

    pub fn cursor_icon(&self) -> CursorIcon {
        match self.hover {
            Some(Ctl::Template) => CursorIcon::Text,
            Some(_) => CursorIcon::Pointer,
            None => CursorIcon::Default,
        }
    }

    fn slider_value(&self, ctl: Ctl, x: f32, cfg: &mut Config) {
        let Some((_, r)) = self.rects.iter().find(|(c, _)| *c == ctl) else { return };
        let t = ((x - r.left()) / r.width()).clamp(0.0, 1.0);
        match ctl {
            Ctl::Dim => cfg.dim = (t * 0.9 * 20.0).round() / 20.0,
            Ctl::Font => cfg.ui_font_size = (12.0 + t * 20.0).round(),
            _ => {}
        }
    }

    pub fn on_move(&mut self, x: f32, y: f32, cfg: &mut Config) -> Fx {
        self.cursor = (x, y);
        self.hover = self.hit(x, y);
        let mut fx = Fx::default();
        if let Some(c) = self.slider {
            self.slider_value(c, x, cfg);
            fx.font = c == Ctl::Font;
        }
        fx
    }

    fn stop_recording(&mut self, fx: &mut Fx) {
        if self.recording.take().is_some() {
            self.record_msg = None;
            fx.record = Some(false);
        }
    }

    pub fn on_press(&mut self, cfg: &mut Config) -> Fx {
        let mut fx = Fx::default();
        let (x, y) = self.cursor;
        let hit = self.hit(x, y);
        if hit != Some(Ctl::Template) {
            self.focus_template = false;
        }
        if !matches!(hit, Some(Ctl::Hotkey(_))) {
            self.stop_recording(&mut fx);
        }
        match hit {
            Some(Ctl::Autostart) => {
                cfg.autostart = !cfg.autostart;
                fx.autostart = Some(cfg.autostart);
                fx.save = true;
            }
            Some(Ctl::Notify) => {
                cfg.notify = !cfg.notify;
                fx.save = true;
            }
            Some(Ctl::SaveOnCopy) => {
                cfg.save_on_copy = !cfg.save_on_copy;
                fx.save = true;
            }
            Some(Ctl::DirChange) => fx.choose_dir = true,
            Some(Ctl::DirOpen) => fx.open_dir = true,
            Some(Ctl::Template) => self.focus_template = true,
            Some(Ctl::Hotkey(i)) => {
                if self.recording != Some(i) {
                    self.stop_recording(&mut fx);
                    self.recording = Some(i);
                    self.record_msg = None;
                    fx.record = Some(true);
                }
            }
            Some(c @ (Ctl::Dim | Ctl::Font)) => {
                self.slider = Some(c);
                self.slider_value(c, x, cfg);
                fx.font = c == Ctl::Font;
            }
            Some(Ctl::OpenConfig) => fx.open_config = true,
            Some(Ctl::Reset) => fx.reset = true,
            Some(Ctl::Close) => fx.close = true,
            Some(Ctl::ShellRegister) => fx.shell_register = Some(true),
            Some(Ctl::ShellUnregister) => fx.shell_register = Some(false),
            Some(Ctl::DefaultApps) => fx.default_apps = true,
            Some(Ctl::KeyboardSettings) => fx.keyboard_settings = true,
            None => {}
        }
        fx
    }

    pub fn on_release(&mut self) -> Fx {
        let mut fx = Fx::default();
        if self.slider.take().is_some() {
            fx.save = true;
        }
        fx
    }

    /// pressed=false приходит только для PrintScreen: Windows не шлёт для неё key down.
    pub fn on_key(&mut self, code: Option<KeyCode>, named: Option<NamedKey>, text: Option<&str>, pressed: bool, cfg: &mut Config) -> Fx {
        let mut fx = Fx::default();
        if let Some(i) = self.recording {
            let is_prtsc = code == Some(KeyCode::PrintScreen);
            if !pressed && !is_prtsc {
                return fx;
            }
            if pressed && is_prtsc {
                return fx;
            }
            let no_mods = !(self.mods.ctrl || self.mods.alt || self.mods.shift);
            match (named, no_mods) {
                (Some(NamedKey::Escape), true) => {
                    self.stop_recording(&mut fx);
                    return fx;
                }
                (Some(NamedKey::Backspace | NamedKey::Delete), true) => {
                    if i == 0 {
                        self.record_msg = Some("Основной хоткей нельзя очистить".into());
                        return fx;
                    }
                    cfg.fallback_hotkey.clear();
                    self.stop_recording(&mut fx);
                    fx.hotkeys = true;
                    fx.save = true;
                    return fx;
                }
                _ => {}
            }
            let Some(name) = code.and_then(key_name) else { return fx };
            let single_char = name.chars().count() == 1;
            if no_mods && single_char {
                self.record_msg = Some("Для буквы или цифры нужен Ctrl, Alt или Shift".into());
                return fx;
            }
            let mut parts: Vec<&str> = Vec::new();
            if self.mods.ctrl {
                parts.push("Ctrl");
            }
            if self.mods.alt {
                parts.push("Alt");
            }
            if self.mods.shift {
                parts.push("Shift");
            }
            parts.push(&name);
            let combo = parts.join("+");
            if combo.parse::<HotKey>().is_err() {
                self.record_msg = Some(format!("Клавиша {name} не поддерживается"));
                return fx;
            }
            // PrintScreen занят Windows (параметр «PrintScreen открывает захват экрана»):
            // регистрируемся обработчиком и открываем выбор программы по умолчанию.
            if combo == "PrintScreen" && self.shell.supported && self.shell.key_enabled && !self.shell.frostshot_is_handler() {
                fx.shell_register = Some(true);
            }
            if i == 0 {
                cfg.hotkey = combo;
            } else {
                cfg.fallback_hotkey = combo;
            }
            self.stop_recording(&mut fx);
            fx.hotkeys = true;
            fx.save = true;
            return fx;
        }
        if !pressed {
            return fx;
        }
        if self.focus_template {
            match named {
                Some(NamedKey::Escape | NamedKey::Enter | NamedKey::Tab) => self.focus_template = false,
                Some(NamedKey::Backspace) => {
                    self.template.pop();
                }
                _ => {
                    if let (Some(t), false) = (text, self.mods.ctrl && !self.mods.alt) {
                        self.template.extend(t.chars().filter(|c| !c.is_control()));
                    }
                }
            }
            if config::file_name(&self.template).is_ok() && self.template != cfg.file_template {
                cfg.file_template = self.template.clone();
                fx.save = true;
            }
            return fx;
        }
        if named == Some(NamedKey::Escape) {
            fx.close = true;
        }
        fx
    }

    /// Нарисовать и вывести окно.
    pub fn present(&mut self, cfg: &Config) {
        let size = self.window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        if self.pm.width() != w || self.pm.height() != h {
            self.pm = Pixmap::new(w, h).unwrap();
        }
        let content_h = self.draw(cfg);
        // Подогнать высоту окна под содержимое (меняется вместе с размером шрифта).
        {
            self.sized = true;
            let s = self.window.scale_factor();
            let want = (content_h as f64 / s).ceil();
            if (want - size.height as f64 / s).abs() > 2.0 {
                let _ = self.window.request_inner_size(LogicalSize::new(WIDTH, want));
            }
        }
        let (Some(nw), Some(nh)) = (std::num::NonZeroU32::new(w), std::num::NonZeroU32::new(h)) else { return };
        if self.surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = self.surface.buffer_mut() else { return };
        for (dst, px) in buf.iter_mut().zip(self.pm.data().chunks_exact(4)) {
            *dst = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        }
        let _ = buf.present();
    }

    fn draw(&mut self, cfg: &Config) -> f32 {
        let s = self.window.scale_factor() as f32;
        let pm = &mut self.pm;
        pm.fill(tiny_skia::Color::from_rgba8(BG[0], BG[1], BG[2], 255));
        self.rects.clear();
        let Some(font) = self.font.clone() else { return 400.0 * s };
        let font = font.as_ref();
        let pad = 24.0 * s;
        let w = pm.width() as f32;
        let cw = w - 2.0 * pad;
        // Все размеры текста от общего параметра ui_font_size.
        let f = crate::ui::ui_font();
        let body = f * s;
        let small = (f - 2.0) * s;
        let row = (f * 2.1).max(34.0) * s;
        let gap_line = (f + 8.0) * s;
        let hover = self.hover;
        let mut y = 18.0 * s;
        let rects = &mut self.rects;

        let text = |pm: &mut Pixmap, t: &str, x: f32, y: f32, size: f32, c: Rgb| {
            draw::draw_text(pm, font, t, x, y, size, c, 1.0, None);
        };
        let tw = |t: &str, size: f32| draw::text_size(font, t, size).0;
        let center_y = |top: f32, hgt: f32, size: f32| top + (hgt - draw::line_height(font, size)) / 2.0;
        let button = |pm: &mut Pixmap, rects: &mut Vec<(Ctl, Rect)>, ctl: Ctl, x: f32, y: f32, label: &str, primary: bool| -> f32 {
            let bw = tw(label, body) + 28.0 * s;
            let r = Rect::from_xywh(x, y, bw, row).unwrap();
            let fill = if primary { ACCENT } else if hover == Some(ctl) { FIELD_HOVER } else { FIELD };
            draw::fill_rounded(pm, r, 6.0 * s, fill, 1.0);
            text(pm, label, x + 14.0 * s, center_y(y, row, body), body, if primary { [255, 255, 255] } else { FG });
            rects.push((ctl, r));
            bw
        };
        let section = |pm: &mut Pixmap, y: &mut f32, title: &str| {
            let _ = gap_line;
            *y += 10.0 * s;
            text(pm, title, pad, *y, small, ACCENT);
            *y += small * 2.0;
        };

        let title = (f + 8.0) * s;
        text(pm, "Frostshot", pad, y, title, FG);
        let ver = concat!("версия ", env!("CARGO_PKG_VERSION"));
        text(pm, ver, pad + tw("Frostshot", title) + 12.0 * s, y + (title - small) * 0.6, small, MUTED);
        y += title * 1.8;

        // Общие.
        section(pm, &mut y, "Общие");
        for (ctl, on, label) in [
            (Ctl::Autostart, cfg.autostart, "Запускать при входе в систему"),
            (Ctl::Notify, cfg.notify, "Уведомление после снимка (клик: доработать)"),
            (Ctl::SaveOnCopy, cfg.save_on_copy, "При копировании также сохранять в папку"),
        ] {
            let bs = (f + 2.0) * s;
            let by = y + (row - bs) / 2.0;
            let br = Rect::from_xywh(pad, by, bs, bs).unwrap();
            draw::fill_rounded(pm, br, 4.0 * s, if on { ACCENT } else if hover == Some(ctl) { FIELD_HOVER } else { FIELD }, 1.0);
            if !on {
                if let Some(p) = draw::rounded_rect(br, 4.0 * s) {
                    draw::stroke_path(pm, &p, BORDER, 1.0, 1.0 * s, None);
                }
            } else {
                let (x0, y0, k) = (pad, by, bs / 18.0);
                draw::line(pm, x0 + 4.0 * k, y0 + 9.5 * k, x0 + 7.5 * k, y0 + 13.0 * k, [255, 255, 255], 1.0, 2.0 * s, None);
                draw::line(pm, x0 + 7.5 * k, y0 + 13.0 * k, x0 + 14.0 * k, y0 + 5.5 * k, [255, 255, 255], 1.0, 2.0 * s, None);
            }
            text(pm, label, pad + bs + 12.0 * s, center_y(y, row, body), body, FG);
            let lw = bs + 12.0 * s + tw(label, body);
            rects.push((ctl, Rect::from_xywh(pad, y, lw, row).unwrap()));
            y += row + 4.0 * s;
        }

        // Сохранение.
        section(pm, &mut y, "Сохранение");
        text(pm, "Папка", pad, y, small, MUTED);
        y += gap_line;
        {
            let bx_open = w - pad - (tw("Открыть", body) + 28.0 * s);
            let bx_change = bx_open - 8.0 * s - (tw("Сменить…", body) + 28.0 * s);
            let fw = bx_change - 8.0 * s - pad;
            let fr = Rect::from_xywh(pad, y, fw, row).unwrap();
            draw::fill_rounded(pm, fr, 6.0 * s, FIELD, 1.0);
            let full = cfg.save_dir.display().to_string();
            let avail = fw - 24.0 * s;
            let mut shown = full.clone();
            if tw(&shown, body) > avail {
                let chars: Vec<char> = full.chars().collect();
                let mut start = 0;
                while start < chars.len() {
                    shown = format!("…{}", chars[start..].iter().collect::<String>());
                    if tw(&shown, body) <= avail {
                        break;
                    }
                    start += 1;
                }
            }
            text(pm, &shown, pad + 12.0 * s, center_y(y, row, body), body, FG);
            button(pm, rects, Ctl::DirChange, bx_change, y, "Сменить…", false);
            button(pm, rects, Ctl::DirOpen, bx_open, y, "Открыть", false);
            y += row + 12.0 * s;
        }
        text(pm, "Имя файла: %Y год, %m месяц, %d день, %H-%M-%S время", pad, y, small, MUTED);
        y += gap_line;
        {
            let fr = Rect::from_xywh(pad, y, cw, row).unwrap();
            let focused = self.focus_template;
            draw::fill_rounded(pm, fr, 6.0 * s, if hover == Some(Ctl::Template) && !focused { FIELD_HOVER } else { FIELD }, 1.0);
            if focused {
                if let Some(p) = draw::rounded_rect(fr, 6.0 * s) {
                    draw::stroke_path(pm, &p, ACCENT, 1.0, 1.5 * s, None);
                }
            }
            let ty = center_y(y, row, body);
            text(pm, &self.template, pad + 12.0 * s, ty, body, FG);
            if focused {
                let cx = pad + 12.0 * s + tw(&self.template, body) + 1.0 * s;
                draw::line(pm, cx, ty, cx, ty + draw::line_height(font, body), FG, 1.0, 1.5 * s, None);
            }
            rects.push((Ctl::Template, fr));
            y += row + 6.0 * s;
            match config::file_name(&self.template) {
                Ok(n) => text(pm, &format!("Пример: {n}"), pad, y, small, MUTED),
                Err(e) => text(pm, &e, pad, y, small, ERR),
            }
            y += gap_line;
        }

        // Захват.
        section(pm, &mut y, "Захват");
        let label_w = tw("Запасной хоткей", body) + 24.0 * s;
        for (i, (label, value)) in [("Хоткей", &cfg.hotkey), ("Запасной хоткей", &cfg.fallback_hotkey)].into_iter().enumerate() {
            text(pm, label, pad, center_y(y, row, body), body, FG);
            let fx = pad + label_w;
            let fr = Rect::from_xywh(fx, y, cw - label_w, row).unwrap();
            let rec = self.recording == Some(i);
            draw::fill_rounded(pm, fr, 6.0 * s, if hover == Some(Ctl::Hotkey(i)) && !rec { FIELD_HOVER } else { FIELD }, 1.0);
            if rec {
                if let Some(p) = draw::rounded_rect(fr, 6.0 * s) {
                    draw::stroke_path(pm, &p, ACCENT, 1.0, 1.5 * s, None);
                }
            }
            let (shown, c) = if rec {
                ("Нажмите сочетание…  Esc: отмена".to_string(), MUTED)
            } else if value.is_empty() {
                ("не задан".to_string(), MUTED)
            } else {
                (value.clone(), FG)
            };
            text(pm, &shown, fx + 12.0 * s, center_y(y, row, body), body, c);
            rects.push((Ctl::Hotkey(i), fr));
            y += row + 4.0 * s;
            let msg = if rec { self.record_msg.clone() } else { self.hotkey_err[i].clone() };
            if let Some(m) = msg {
                text(pm, &m, fx, y, small, ERR);
                y += gap_line;
            }
        }
        // PrintScreen через Windows (ms-screenclip).
        if self.shell.supported {
            y += 4.0 * s;
            text(pm, "PrintScreen через Windows", pad, y, body, FG);
            y += gap_line + 2.0 * s;
            let sh = &self.shell;
            let name = match sh.handler.as_deref() {
                Some(crate::platform::SCREENCLIP_PROGID) => "Frostshot".to_string(),
                Some(p) => p.to_string(),
                None => "Ножницы Windows".to_string(),
            };
            let (status, color) = if !sh.key_enabled {
                ("Параметр Windows выключен: PrintScreen ловит хоткей Frostshot".to_string(), MUTED)
            } else if sh.frostshot_is_handler() {
                ("PrintScreen и Win+Shift+S открывают Frostshot".to_string(), ACCENT)
            } else {
                (format!("Сейчас PrintScreen открывает: {name}"), ERR)
            };
            text(pm, &status, pad, y, small, color);
            y += gap_line;
            let mut bx = pad;
            if !sh.registered {
                bx += button(pm, rects, Ctl::ShellRegister, bx, y, "Зарегистрировать Frostshot", false) + 8.0 * s;
            } else {
                if !sh.frostshot_is_handler() {
                    bx += button(pm, rects, Ctl::DefaultApps, bx, y, "Выбрать Frostshot в Windows…", false) + 8.0 * s;
                }
                bx += button(pm, rects, Ctl::ShellUnregister, bx, y, "Убрать регистрацию", false) + 8.0 * s;
            }
            button(pm, rects, Ctl::KeyboardSettings, bx, y, "Параметр PrintScreen…", false);
            y += row + 10.0 * s;
        }
        y += 6.0 * s;
        for (ctl, label, t) in [
            (Ctl::Dim, format!("Затемнение вне выделения: {}%", (cfg.dim * 100.0).round()), cfg.dim / 0.9),
            (Ctl::Font, format!("Шрифт подсказок: {} px", cfg.ui_font_size.round()), (cfg.ui_font_size - 12.0) / 20.0),
        ] {
            text(pm, &label, pad, y, body, FG);
            y += gap_line + 4.0 * s;
            let tr = Rect::from_xywh(pad + 8.0 * s, y, cw - 16.0 * s, 24.0 * s).unwrap();
            let cy = y + 12.0 * s;
            let t = t.clamp(0.0, 1.0);
            let kx = tr.left() + tr.width() * t;
            draw::line(pm, tr.left(), cy, tr.right(), cy, BORDER, 1.0, 4.0 * s, None);
            draw::line(pm, tr.left(), cy, kx, cy, ACCENT, 1.0, 4.0 * s, None);
            draw::circle(pm, kx, cy, 8.0 * s, if hover == Some(ctl) || self.slider == Some(ctl) { [255, 255, 255] } else { FG }, 1.0, true, 0.0);
            rects.push((ctl, tr));
            y += 34.0 * s;
        }

        // Клавиши.
        section(pm, &mut y, "Клавиши в редакторе");
        let col = cw / 2.0;
        let lh = small * 1.8;
        let key_w = KEYS.iter().map(|(k, _)| tw(k, small)).fold(0.0f32, f32::max) + 16.0 * s;
        for (i, (k, d)) in KEYS.iter().enumerate() {
            let cx = pad + (i % 2) as f32 * col;
            let cy = y + (i / 2) as f32 * lh;
            text(pm, k, cx, cy, small, FG);
            text(pm, d, cx + key_w, cy, small, MUTED);
        }
        y += KEYS.len().div_ceil(2) as f32 * lh + 16.0 * s;

        // Кнопки.
        let mut bx = pad;
        bx += button(pm, rects, Ctl::OpenConfig, bx, y, "Открыть config.toml", false) + 8.0 * s;
        button(pm, rects, Ctl::Reset, bx, y, "Сбросить", false);
        let done_w = tw("Готово", body) + 28.0 * s;
        button(pm, rects, Ctl::Close, w - pad - done_w, y, "Готово", true);
        y += row + pad;
        y
    }
}
