//! Методы App: оверлей (захват, повторное открытие, проект), вывод результата, уведомление.

use super::*;

/// Что ждёт выбора файла в диалоге сохранения.
pub(crate) enum Pending {
    Png(Pixmap),
    /// Байты проекта и миниатюра для уведомления.
    Project(Vec<u8>, Pixmap),
}

impl App {
    fn busy(&self) -> bool {
        self.overlay.is_some() || self.pending_save.is_some()
    }

    /// Окна-оверлеи на прямоугольниках мониторов (глобальные физические координаты).
    fn create_windows(&self, el: &ActiveEventLoop, rects: &[(i32, i32, u32, u32)]) -> Option<(Vec<OverlayWin>, Vec<f32>)> {
        let monitors: Vec<_> = el.available_monitors().collect();
        let mut wins = Vec::new();
        let mut scales = Vec::new();
        for (i, &(x, y, w, h)) in rects.iter().enumerate() {
            let handle = match_monitor(&monitors, x, y, w, h);
            let mut attrs = Window::default_attributes()
                .with_title("Frostshot")
                .with_decorations(false)
                .with_resizable(false)
                .with_visible(false)
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_position(PhysicalPosition::new(x, y))
                .with_inner_size(PhysicalSize::new(w, h));
            if let Some(hm) = &handle {
                attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(Some(hm.clone()))));
            }
            attrs = platform::overlay_attributes(attrs);
            let window = match el.create_window(attrs) {
                Ok(w) => Rc::new(w),
                Err(e) => {
                    log::error!("create window: {e}");
                    return None;
                }
            };
            let surface = match softbuffer::Context::new(window.clone()).and_then(|c| Surface::new(&c, window.clone())) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("softbuffer: {e}");
                    return None;
                }
            };
            let scale = handle.as_ref().map(|h| h.scale_factor()).unwrap_or(1.0) as f32;
            log::info!("monitor {i}: {x},{y} {w}x{h} scale {scale}");
            scales.push(scale);
            wins.push(OverlayWin { window, surface, mon: i, pos: (0.0, 0.0) });
        }
        Some((wins, scales))
    }

    fn show_overlay(&mut self, mut session: Session, mut wins: Vec<OverlayWin>, reopened: bool) {
        let rects = session.monitor_rects();
        let cursor = platform::cursor_pos().and_then(|(gx, gy)| {
            rects
                .iter()
                .position(|&(x, y, w, h)| gx >= x && gy >= y && gx < x + w as i32 && gy < y + h as i32)
                .map(|i| (i, (gx - rects[i].0) as f32, (gy - rects[i].1) as f32))
        });
        if let Some((m, x, y)) = cursor {
            session.on_move(m, x, y);
            if let Some(w) = wins.iter_mut().find(|w| w.mon == m) {
                w.pos = (x, y);
            }
        }
        let mut ov = Overlay { wins, session, reopened };
        for w in &mut ov.wins {
            present(w, &mut ov.session);
            w.window.set_visible(true);
            w.window.set_cursor(ov.session.cursor_icon(w.mon));
        }
        let focus = cursor.map(|c| c.0).or_else(|| ov.session.active_rect().map(|_| 0)).unwrap_or(0);
        if let Some(w) = ov.wins.iter().find(|w| w.mon == focus) {
            platform::force_foreground(&w.window);
        }
        self.overlay = Some(ov);
    }

    pub(crate) fn start_capture(&mut self, el: &ActiveEventLoop) {
        if self.busy() {
            return;
        }
        self.toast = None;
        self.popup = None;
        let t0 = std::time::Instant::now();
        let shots = match capture::capture_all() {
            Ok(s) => s,
            Err(e) => {
                log::error!("capture failed: {e}");
                return;
            }
        };
        log::info!("captured {} monitors in {:?}", shots.len(), t0.elapsed());
        // «Пуск» и другие панели оболочки уже в кадре; закрываем их, иначе они останутся поверх оверлея.
        platform::dismiss_shell_flyout();
        let rects: Vec<_> = shots.iter().map(|s| (s.x, s.y, s.width(), s.height())).collect();
        let Some((wins, scales)) = self.create_windows(el, &rects) else { return };
        let session = Session::new(shots, scales, self.config.dim, self.config.color, self.config.width, self.font.clone());
        self.show_overlay(session, wins, false);
        log::info!("overlay shown in {:?}", t0.elapsed());
    }

    /// Открыть последний снимок с редактируемой разметкой.
    pub(crate) fn reopen_last(&mut self, el: &ActiveEventLoop) {
        if self.busy() {
            return;
        }
        platform::dismiss_shell_flyout();
        let Some(mut session) = self.last.take() else { return };
        self.toast = None;
        session.wake();
        let rects = session.monitor_rects();
        match self.create_windows(el, &rects) {
            Some((wins, _)) => self.show_overlay(session, wins, true),
            None => self.last = Some(session),
        }
        self.update_tray_last();
    }

    pub(crate) fn open_project(&mut self, el: &ActiveEventLoop, path: &std::path::Path) {
        if self.busy() {
            return;
        }
        platform::dismiss_shell_flyout();
        let res = std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| {
            let (h, _) = project::decode(&bytes)?;
            // Монитор под курсором, если снимок в него помещается, иначе самый большой подходящий.
            let monitors: Vec<_> = el.available_monitors().collect();
            let cur = platform::cursor_pos();
            let fits = |m: &&winit::monitor::MonitorHandle| m.size().width >= h.width && m.size().height >= h.height;
            let under_cursor = |m: &&winit::monitor::MonitorHandle| {
                cur.is_some_and(|(x, y)| {
                    let (p, s) = (m.position(), m.size());
                    x >= p.x && y >= p.y && x < p.x + s.width as i32 && y < p.y + s.height as i32
                })
            };
            let area = |m: &&winit::monitor::MonitorHandle| m.size().width as u64 * m.size().height as u64;
            let m = monitors
                .iter()
                .filter(fits)
                .find(under_cursor)
                .or_else(|| monitors.iter().filter(fits).max_by_key(area))
                .or_else(|| monitors.iter().max_by_key(area))
                .ok_or("нет мониторов")?;
            if !fits(&m) {
                log::warn!("project {}x{} larger than any monitor, cropped view", h.width, h.height);
            }
            let (p, s) = (m.position(), m.size());
            let session = Session::from_project(&bytes, p.x, p.y, m.scale_factor() as f32, self.config.dim, self.font.clone())?;
            Ok((session, (p.x, p.y, s.width, s.height)))
        });
        match res {
            Ok((session, rect)) => {
                self.toast = None;
                if let Some((wins, _)) = self.create_windows(el, &[rect]) {
                    self.show_overlay(session, wins, true);
                    log::info!("project opened {}", path.display());
                }
            }
            Err(e) => {
                log::error!("open project {}: {e}", path.display());
                let msg = format!("Не удалось открыть {}:\n{e}", path.display());
                std::thread::spawn(move || {
                    rfd::MessageDialog::new().set_title("Frostshot").set_description(msg).set_level(rfd::MessageLevel::Error).show();
                });
            }
        }
    }

    pub(crate) fn pick_project(&self) {
        let proxy = self.proxy.clone();
        let dir = self.config.save_dir.clone();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .set_title("Открыть проект Frostshot")
                .set_directory(&dir)
                .add_filter("Проект Frostshot", &[project::EXT])
                .pick_file();
            let _ = proxy.send_event(UserEvent::ProjectChosen(path));
        });
    }

    fn save_style(&mut self, session: &Session) {
        if session.color != self.config.color || session.width != self.config.width {
            self.config.color = session.color;
            self.config.width = session.width;
            self.config.save();
        }
    }

    /// Esc/закрытие без результата. Повторно открытый снимок остаётся последним.
    pub(crate) fn close_overlay(&mut self) {
        if let Some(ov) = self.overlay.take() {
            let mut session = ov.session;
            self.save_style(&session);
            if ov.reopened {
                session.hibernate();
                self.last = Some(session);
                self.last_at = std::time::Instant::now();
            }
        }
        self.pending_save = None;
        self.update_tray_last();
    }

    /// Результат получен: оверлей закрываем, сессию храним, показываем уведомление.
    fn finish_overlay(&mut self, el: &ActiveEventLoop, title: Option<String>, thumb: &Pixmap) {
        self.pending_save = None;
        let Some(ov) = self.overlay.take() else { return };
        let mut session = ov.session;
        self.save_style(&session);
        let rect = session.active_rect();
        if self.config.history {
            // Проект в историю: PNG кодируется в фоне, меню трея обновится по событию.
            if let Some((header, shot)) = session.project_parts() {
                let (thumb, proxy) = (thumb.clone(), self.proxy.clone());
                let (max, days) = (self.config.history_max, self.config.history_days);
                std::thread::spawn(move || {
                    match crate::project::build(header, &shot).and_then(|b| crate::history::save(&b, &thumb)) {
                        Ok(p) => log::info!("history: {}", p.display()),
                        Err(e) => log::warn!("history: {e}"),
                    }
                    crate::history::prune(max, days);
                    let _ = proxy.send_event(UserEvent::HistoryChanged);
                });
            }
        }
        session.hibernate();
        self.last = Some(session);
        self.last_at = std::time::Instant::now();
        self.update_tray_last();
        let Some(title) = title else { return };
        if !self.config.notify {
            return;
        }
        let (cx, cy) = rect.map(|(x, y, w, h)| (x + w as i32 / 2, y + h as i32 / 2)).unwrap_or((0, 0));
        let area = platform::work_area(cx, cy).or_else(|| rect.map(|(x, y, w, h)| (x, y, x + w as i32, y + h as i32 - 48)));
        let Some(area) = area else { return };
        let scale = el
            .available_monitors()
            .find(|m| {
                let (p, s) = (m.position(), m.size());
                cx >= p.x && cy >= p.y && cx < p.x + s.width as i32 && cy < p.y + s.height as i32
            })
            .map(|m| m.scale_factor() as f32)
            .unwrap_or(1.0);
        match toast::Toast::open(el, self.font.clone(), thumb, title, "Нажмите, чтобы доработать".into(), area, scale) {
            Ok(t) => self.toast = Some(t),
            Err(e) => log::error!("toast: {e}"),
        }
    }

    fn update_tray_last(&self) {
        if let Some(t) = &self.tray {
            t.set_last_enabled(self.last.is_some());
        }
    }

    fn save_dialog(&self, title: &str, name: String, filter: (&'static str, &'static str)) {
        let dir = self.config.save_dir.clone();
        let _ = std::fs::create_dir_all(&dir);
        let proxy = self.proxy.clone();
        let title = title.to_string();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .set_title(title)
                .set_directory(&dir)
                .set_file_name(name)
                .add_filter(filter.0, &[filter.1])
                .save_file();
            let _ = proxy.send_event(UserEvent::SaveChosen(path));
        });
    }

    fn hide_overlay(&self) {
        // Оверлей поверх всех окон перекрыл бы диалог: прячем на время выбора файла.
        if let Some(ov) = &self.overlay {
            for w in &ov.wins {
                w.window.set_visible(false);
            }
        }
    }

    pub(crate) fn handle_action(&mut self, el: &ActiveEventLoop, action: Action) {
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
                let mut title = "Скопировано в буфер".to_string();
                if self.config.save_on_copy {
                    let path = self.config.save_dir.join(output::default_file_name(&self.config.file_template));
                    match output::save_png(&img, &path) {
                        Ok(p) => {
                            log::info!("saved {}", p.display());
                            title = "Скопировано и сохранено".into();
                        }
                        Err(e) => log::error!("save: {e}"),
                    }
                }
                self.finish_overlay(el, Some(title), &img);
            }
            Action::Pin => {
                let Some(ov) = self.overlay.as_mut() else { return };
                let (Some(img), Some((x, y, scale))) = (ov.session.result(), ov.session.selection_origin()) else { return };
                let thumb = img.clone();
                match pin::Pin::open(el, img, x, y, scale) {
                    Ok(p) => {
                        log::info!("pinned {}x{} at {x},{y}", thumb.width(), thumb.height());
                        self.finish_overlay(el, None, &thumb);
                        self.pins.push(p);
                    }
                    Err(e) => log::error!("pin: {e}"),
                }
            }
            Action::CopyText => self.start_ocr(OcrPurpose::CopyText),
            Action::SelectionDone => {
                if self.config.auto_hide {
                    self.start_ocr(OcrPurpose::AutoHideQuiet);
                }
            }
            Action::AutoHide => self.start_ocr(OcrPurpose::AutoHide),
            Action::QuickSave => {
                let Some(img) = self.overlay.as_mut().and_then(|o| o.session.result()) else { return };
                let path = self.config.save_dir.join(output::default_file_name(&self.config.file_template));
                match output::save_png(&img, &path) {
                    Ok(p) => {
                        log::info!("saved {}", p.display());
                        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        self.finish_overlay(el, Some(format!("Сохранено: {name}")), &img);
                    }
                    Err(e) => {
                        log::error!("save: {e}");
                        self.close_overlay();
                    }
                }
            }
            Action::Save => {
                let Some(img) = self.overlay.as_mut().and_then(|o| o.session.result()) else { return };
                self.hide_overlay();
                self.pending_save = Some(Pending::Png(img));
                let name = output::default_file_name(&self.config.file_template);
                self.save_dialog("Сохранить скриншот", name, ("PNG", "png"));
            }
            Action::SaveProject => {
                let Some(ov) = self.overlay.as_mut() else { return };
                let (bytes, thumb) = match (ov.session.to_project(), ov.session.result()) {
                    (Ok(b), Some(t)) => (b, t),
                    (Err(e), _) => {
                        log::error!("project: {e}");
                        return;
                    }
                    _ => return,
                };
                self.hide_overlay();
                self.pending_save = Some(Pending::Project(bytes, thumb));
                let name = std::path::Path::new(&output::default_file_name(&self.config.file_template))
                    .with_extension(project::EXT)
                    .to_string_lossy()
                    .to_string();
                self.save_dialog(
                    "Сохранить проект (исходный снимок без пикселизации, не для отправки)",
                    name,
                    ("Проект Frostshot", project::EXT),
                );
            }
        }
    }

    /// Распознать выделение в фоновом потоке: оверлей не подвисает.
    fn start_ocr(&mut self, purpose: OcrPurpose) {
        let Some(ov) = self.overlay.as_mut() else { return };
        if ov.session.ocr_busy {
            return;
        }
        let Some((crop, offset)) = ov.session.ocr_source() else { return };
        ov.session.ocr_busy = true;
        if !matches!(purpose, OcrPurpose::AutoHideQuiet) {
            ov.session.set_status("Распознаю текст…", true);
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let result = platform::ocr_recognize(&crop);
            log::info!("ocr {}x{} in {:?}: {}", crop.width(), crop.height(), t0.elapsed(), match &result {
                Ok(l) => format!("{} lines", l.len()),
                Err(e) => e.clone(),
            });
            let _ = proxy.send_event(UserEvent::OcrDone(purpose, offset, result));
        });
    }

    pub(crate) fn on_ocr_done(&mut self, el: &ActiveEventLoop, purpose: OcrPurpose, offset: (f32, f32), result: Result<Vec<ocr::Line>, String>) {
        // Оверлей могли закрыть, пока шло распознавание.
        let Some(ov) = self.overlay.as_mut() else { return };
        ov.session.ocr_busy = false;
        let lines = match result {
            Ok(l) => l,
            Err(e) => {
                if !matches!(purpose, OcrPurpose::AutoHideQuiet) {
                    ov.session.set_status(e, false);
                }
                return;
            }
        };
        match purpose {
            OcrPurpose::AutoHide | OcrPurpose::AutoHideQuiet => {
                let quiet = matches!(purpose, OcrPurpose::AutoHideQuiet);
                let found = ocr::find_sensitive(&lines, 3.0);
                let rects: Vec<_> = found
                    .iter()
                    .filter_map(|(_, r)| tiny_skia::Rect::from_xywh(r.x() + offset.0, r.y() + offset.1, r.width(), r.height()))
                    .collect();
                let added = ov.session.apply_hide(&rects);
                if !quiet || added > 0 {
                    let text = ocr::summary(&found);
                    ov.session.set_status(if quiet { format!("Автоскрытие: {text}") } else { text }, false);
                }
            }
            OcrPurpose::CopyText => {
                let text = ocr::text_of(&lines);
                if text.trim().is_empty() {
                    ov.session.set_status("Текст не найден", false);
                    return;
                }
                let n = lines.len();
                let thumb = ov.session.result();
                if self.clipboard.is_none() {
                    self.clipboard = arboard::Clipboard::new().ok();
                }
                match self.clipboard.as_mut().map(|cb| cb.set_text(text)) {
                    Some(Ok(())) => {
                        let title = format!("Текст скопирован: {n} {}", plural(n, "строка", "строки", "строк"));
                        match thumb {
                            Some(t) => self.finish_overlay(el, Some(title), &t),
                            None => self.close_overlay(),
                        }
                    }
                    _ => {
                        if let Some(ov) = self.overlay.as_mut() {
                            ov.session.set_status("Не удалось записать в буфер обмена", false);
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn on_save_chosen(&mut self, el: &ActiveEventLoop, path: Option<PathBuf>) {
        let Some(pending) = self.pending_save.take() else { return };
        let Some(path) = path else {
            // Отмена: возвращаем оверлей как был.
            if let Some(ov) = &mut self.overlay {
                for w in &mut ov.wins {
                    w.window.set_visible(true);
                    present(w, &mut ov.session);
                }
                if let Some(w) = ov.wins.first() {
                    platform::force_foreground(&w.window);
                }
            }
            return;
        };
        let (res, thumb, what) = match &pending {
            Pending::Png(img) => (output::save_png(img, &path), img, "Сохранено"),
            Pending::Project(bytes, thumb) => {
                let mut p = path.clone();
                if p.extension().is_none_or(|e| !e.eq_ignore_ascii_case(project::EXT)) {
                    p.set_extension(project::EXT);
                }
                let r = std::fs::write(&p, bytes).map(|_| p).map_err(|e| e.to_string());
                (r, thumb, "Проект сохранён")
            }
        };
        match res {
            Ok(p) => {
                log::info!("saved {}", p.display());
                if let Some(dir) = p.parent() {
                    self.config.save_dir = dir.to_path_buf();
                    self.config.save();
                }
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let thumb = thumb.clone();
                self.finish_overlay(el, Some(format!("{what}: {name}")), &thumb);
            }
            Err(e) => {
                log::error!("save: {e}");
                self.close_overlay();
            }
        }
    }

    pub(crate) fn handle_toast_event(&mut self, el: &ActiveEventLoop, event: WindowEvent) {
        let Some(t) = &mut self.toast else { return };
        match event {
            WindowEvent::RedrawRequested => t.present(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                t.anchor();
                t.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                t.on_move(position.x as f32, position.y as f32);
                self.toast_pos = (position.x as f32, position.y as f32);
            }
            WindowEvent::CursorEntered { .. } => t.set_hovered(true),
            WindowEvent::CursorLeft { .. } => t.set_hovered(false),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                match t.on_click(self.toast_pos.0, self.toast_pos.1) {
                    toast::ToastClick::Open => self.reopen_last(el),
                    toast::ToastClick::Close => self.toast = None,
                }
            }
            _ => {}
        }
    }

    pub(crate) fn handle_overlay_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
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
                    (MouseButton::Middle, st) => {
                        s.on_middle(mon, st == ElementState::Pressed, x, y);
                        Action::None
                    }
                    _ => Action::None,
                };
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                if d != 0.0 {
                    if s.mods.ctrl {
                        let (x, y) = ov.wins[idx].pos;
                        s.on_zoom(mon, d, x, y);
                    } else {
                        s.on_wheel(d);
                    }
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
        self.handle_action(el, action);
    }
}

/// Монитор winit для прямоугольника: точное совпадение позиции, иначе наибольшее пересечение.
fn match_monitor(monitors: &[winit::monitor::MonitorHandle], x: i32, y: i32, w: u32, h: u32) -> Option<winit::monitor::MonitorHandle> {
    if let Some(m) = monitors.iter().find(|m| m.position() == PhysicalPosition::new(x, y)) {
        return Some(m.clone());
    }
    let (sx0, sy0) = (x as i64, y as i64);
    let (sx1, sy1) = (sx0 + w as i64, sy0 + h as i64);
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
    log::warn!("monitor at {x},{y} {w}x{h}: no exact winit match, fallback {:?}", best.as_ref().map(|m| (m.position(), m.size())));
    best
}

pub(crate) fn present(w: &mut OverlayWin, s: &mut Session) {
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

/// 1 строка, 2 строки, 5 строк.
fn plural(n: usize, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    match (n % 10, n % 100) {
        (1, x) if x != 11 => one,
        (2..=4, x) if !(12..=14).contains(&x) => few,
        _ => many,
    }
}
