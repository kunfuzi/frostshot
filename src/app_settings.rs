//! Методы App: хоткеи, автозапуск, окно настроек.

use super::*;

impl App {
    /// (Пере)регистрация хоткеев из конфига. Возвращает подпись для меню и ошибки по полям.
    pub(crate) fn register_hotkeys(&mut self) -> (String, [Option<String>; 2]) {
        let mut errs: [Option<String>; 2] = [None, None];
        let mut label = String::new();
        self.hotkey_ids.clear();
        if let Some(mgr) = &self.hotkeys {
            if !self.registered.is_empty() {
                let _ = mgr.unregister_all(&self.registered);
                self.registered.clear();
            }
            for (i, s) in [self.config.hotkey.clone(), self.config.fallback_hotkey.clone()].into_iter().enumerate() {
                if s.is_empty() {
                    continue;
                }
                match s.parse::<HotKey>() {
                    Ok(hk) => match mgr.register(hk) {
                        Ok(()) => {
                            self.hotkey_ids.push(hk.id());
                            self.registered.push(hk);
                            if label.is_empty() {
                                label = s.clone();
                            }
                            log::info!("hotkey registered: {s}");
                        }
                        Err(e) => {
                            log::warn!("hotkey {s} not registered: {e}");
                            errs[i] = Some("Занято другой программой".into());
                        }
                    },
                    Err(e) => {
                        log::warn!("hotkey {s} parse error: {e}");
                        errs[i] = Some("Неверное сочетание".into());
                    }
                }
            }
        }
        if label.is_empty() {
            label = "клик по иконке".into();
        }
        if let Some(t) = &self.tray {
            t.set_hotkey_label(&label);
        }
        if let Some(st) = &mut self.settings {
            st.hotkey_err = errs.clone();
        }
        (label, errs)
    }

    pub(crate) fn unregister_hotkeys(&mut self) {
        if let Some(mgr) = &self.hotkeys {
            let _ = mgr.unregister_all(&self.registered);
        }
        self.registered.clear();
        self.hotkey_ids.clear();
    }

    pub(crate) fn open_settings(&mut self, el: &ActiveEventLoop) {
        if let Some(st) = &self.settings {
            st.window.set_minimized(false);
            st.window.focus_window();
            return;
        }
        match settings::Settings::open(el, self.font.clone(), &self.config) {
            Ok(st) => {
                self.settings = Some(st);
                self.register_hotkeys();
                if let Some(st) = &self.settings {
                    st.window.request_redraw();
                }
            }
            Err(e) => log::error!("settings window: {e}"),
        }
    }

    pub(crate) fn set_autostart(&mut self, on: bool) {
        match platform::set_autostart(on) {
            Ok(()) => self.config.autostart = on,
            Err(e) => {
                log::error!("autostart: {e}");
                self.config.autostart = platform::autostart_enabled();
            }
        }
        self.config.save();
        if let Some(t) = &self.tray {
            t.set_autostart(self.config.autostart);
        }
    }

    pub(crate) fn apply_fx(&mut self, fx: settings::Fx) {
        if fx.reset {
            let warned = self.config.printscreen_warned;
            self.config = Config::default();
            self.config.printscreen_warned = warned;
            self.set_autostart(false);
            ui::set_font_size(self.config.ui_font_size);
            self.register_hotkeys();
            if let Some(st) = &mut self.settings {
                st.reload(&self.config);
            }
            self.config.save();
        }
        if let Some(on) = fx.autostart {
            self.set_autostart(on);
        }
        match fx.record {
            Some(true) => self.unregister_hotkeys(),
            Some(false) if !fx.hotkeys => {
                self.register_hotkeys();
            }
            _ => {}
        }
        if fx.hotkeys {
            self.register_hotkeys();
        }
        if fx.font {
            ui::set_font_size(self.config.ui_font_size);
        }
        if fx.save {
            self.config.save();
        }
        if fx.choose_dir {
            let dir = self.config.save_dir.clone();
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                let path = rfd::FileDialog::new().set_title("Папка для снимков").set_directory(&dir).pick_folder();
                let _ = proxy.send_event(UserEvent::DirChosen(path));
            });
        }
        if let Some(on) = fx.shell_register {
            match platform::shell_register(on) {
                Ok(()) => {
                    log::info!("shell integration {}", if on { "registered" } else { "removed" });
                    // Выбор программы по умолчанию делает пользователь: Windows не даёт менять его из кода.
                    if on {
                        platform::open_default_apps();
                    }
                }
                Err(e) => log::error!("shell integration: {e}"),
            }
            if let Some(st) = &mut self.settings {
                st.refresh_shell();
            }
        }
        if fx.default_apps {
            platform::open_default_apps();
        }
        if fx.keyboard_settings {
            platform::open_keyboard_settings();
        }
        if fx.open_dir {
            platform::open_folder(&self.config.save_dir);
        }
        if fx.open_config {
            self.config.save();
            if let Some(p) = config::config_path() {
                platform::open_file(&p);
            }
        }
        if fx.close {
            if self.settings.as_ref().is_some_and(|s| s.recording.is_some()) {
                self.settings = None;
                self.register_hotkeys();
            }
            self.settings = None;
        }
        if let Some(st) = &self.settings {
            st.window.request_redraw();
        }
    }

    pub(crate) fn handle_settings_event(&mut self, event: WindowEvent) {
        let Some(st) = &mut self.settings else { return };
        let cfg = &mut self.config;
        let fx = match event {
            WindowEvent::RedrawRequested => {
                st.present(cfg);
                return;
            }
            WindowEvent::CloseRequested => settings::Fx { close: true, ..Default::default() },
            WindowEvent::CursorMoved { position, .. } => {
                let fx = st.on_move(position.x as f32, position.y as f32, cfg);
                st.window.set_cursor(st.cursor_icon());
                fx
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => st.on_press(cfg),
                ElementState::Released => st.on_release(),
            },
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                st.mods = session::Mods { shift: s.shift_key(), ctrl: s.control_key(), alt: s.alt_key() };
                return;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let code = match event.physical_key {
                    PhysicalKey::Code(c) => Some(c),
                    _ => None,
                };
                let named = match &event.logical_key {
                    Key::Named(n) => Some(*n),
                    _ => None,
                };
                st.on_key(code, named, event.text.as_deref(), event.state == ElementState::Pressed, cfg)
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => settings::Fx::default(),
            // Вернулись из параметров Windows: обновить статус PrintScreen.
            WindowEvent::Focused(true) => {
                st.refresh_shell();
                return;
            }
            _ => return,
        };
        self.apply_fx(fx);
    }

}
