use tiny_skia::{Pixmap, Rect};
use std::path::PathBuf;
use tray_icon::menu::{CheckMenuItem, IconMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    icon: TrayIcon,
    capture: MenuItem,
    last: MenuItem,
    history: Submenu,
    /// Пункт истории -> файл проекта.
    pub history_items: Vec<(MenuId, PathBuf)>,
    pub history_folder_id: MenuId,
    pub history_clear_id: MenuId,
    autostart: CheckMenuItem,
    pub capture_id: MenuId,
    pub last_id: MenuId,
    pub open_project_id: MenuId,
    pub folder_id: MenuId,
    pub settings_id: MenuId,
    pub autostart_id: MenuId,
    pub quit_id: MenuId,
}

/// Иконка: крупный курсор мыши по диагонали (остриё слева вверху), в двух других
/// углах уголки рамки выделения. Половина курсора голубее, как грань льда.
/// Рисуется в сетке 128x128 и масштабируется; в мелких размерах линии толще.
pub fn icon_pixmap(size: u32) -> Pixmap {
    use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Point, SpreadMode, Transform};
    let mut pm = Pixmap::new(size, size).unwrap();
    let s = size as f32;
    let k = s / 128.0;
    let small = size <= 24;
    let p = |x: f32, y: f32| (x * k, y * k);
    let fill = |pm: &mut Pixmap, pts: &[(f32, f32)], c: [u8; 3]| {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for q in &pts[1..] {
            pb.line_to(q.0, q.1);
        }
        pb.close();
        if let Some(path) = pb.finish() {
            pm.fill_path(&path, &crate::draw::paint(c, 1.0), FillRule::Winding, Transform::identity(), None);
        }
    };

    // Фон: яркий голубой слева сверху, глубокий синий справа снизу.
    if let Some(path) = crate::draw::rounded_rect(Rect::from_xywh(0.0, 0.0, s, s).unwrap(), 28.0 * k) {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        paint.shader = LinearGradient::new(
            Point::from_xy(0.0, 0.0),
            Point::from_xy(s, s),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(0x1c, 0x9c, 0xf5, 255)),
                GradientStop::new(1.0, Color::from_rgba8(0x0b, 0x3c, 0xa8, 255)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap_or(tiny_skia::Shader::SolidColor(Color::from_rgba8(0x16, 0x6c, 0xd8, 255)));
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // Уголки рамки: справа вверху и слева внизу.
    let corner = [0xd8, 0xf0, 0xff];
    let w = if small { 13.0 } else { 10.0 } * k;
    for pts in [[(76.0, 24.0), (104.0, 24.0), (104.0, 52.0)], [(24.0, 76.0), (24.0, 104.0), (52.0, 104.0)]] {
        let mut pb = PathBuilder::new();
        let a = p(pts[0].0, pts[0].1);
        pb.move_to(a.0, a.1);
        for q in &pts[1..] {
            let q = p(q.0, q.1);
            pb.line_to(q.0, q.1);
        }
        if let Some(path) = pb.finish() {
            crate::draw::stroke_path(&mut pm, &path, corner, 1.0, w, None);
        }
    }

    // Курсор мыши по диагонали: классический указатель, повёрнутый так, что его ось
    // (от острия к середине хвоста) идёт под 45°. Правая половина голубее, как грань льда.
    let (tx, ty, sc) = (24.0f32, 20.0f32, 3.0f32);
    let th = (26.5f32.atan2(12.0) - std::f32::consts::FRAC_PI_4).to_degrees().to_radians() * -1.0;
    let (sn, cs) = (th.sin(), th.cos());
    let c = |x: f32, y: f32| p(tx + (x * cs - y * sn) * sc, ty + (x * sn + y * cs) * sc);
    let cursor = [c(0.0, 0.0), c(0.0, 24.0), c(6.0, 18.5), c(10.0, 27.5), c(14.0, 25.5), c(10.2, 17.0), c(17.0, 17.0)];
    fill(&mut pm, &cursor, [0xff, 0xff, 0xff]);
    fill(&mut pm, &[c(0.0, 0.0), c(17.0, 17.0), c(10.2, 17.0), c(14.0, 25.5), c(12.0, 26.5)], [0xc6, 0xe8, 0xff]);
    pm
}

pub fn rgba_straight(pm: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.data().len());
    for p in pm.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

/// ICO из PNG-кадров 16..256 для ресурса exe (`frostshot --write-icon assets/frostshot.ico`).
pub fn write_ico(path: &std::path::Path) -> Result<(), String> {
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let pngs: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| icon_pixmap(s).encode_png().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, png) in sizes.iter().zip(&pngs) {
        let b = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[b, b, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &pngs {
        out.extend_from_slice(png);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, out).map_err(|e| e.to_string())
}

fn capture_label(hotkey: &str) -> String {
    format!("Сделать скриншот ({hotkey})")
}

impl Tray {
    pub fn build(hotkey_label: &str, autostart: bool) -> Result<Tray, String> {
        let capture = MenuItem::new(capture_label(hotkey_label), true, None);
        let last = MenuItem::new("Открыть последний снимок", false, None);
        let history = Submenu::new("История", true);
        let open_project = MenuItem::new("Открыть проект…", true, None);
        let folder = MenuItem::new("Открыть папку со снимками", true, None);
        let settings = MenuItem::new("Настройки…", true, None);
        let auto = CheckMenuItem::new("Запускать при входе в систему", true, autostart, None);
        let quit = MenuItem::new("Выход", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &capture,
            &last,
            &history,
            &open_project,
            &folder,
            &PredefinedMenuItem::separator(),
            &settings,
            &auto,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .map_err(|e| e.to_string())?;

        let pm = icon_pixmap(32);
        let icon = Icon::from_rgba(rgba_straight(&pm), 32, 32).map_err(|e| e.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(format!("Frostshot: {hotkey_label}"))
            .with_icon(icon)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Tray {
            icon: tray,
            capture_id: capture.id().clone(),
            last_id: last.id().clone(),
            open_project_id: open_project.id().clone(),
            folder_id: folder.id().clone(),
            settings_id: settings.id().clone(),
            autostart_id: auto.id().clone(),
            quit_id: quit.id().clone(),
            capture,
            last,
            history,
            history_items: Vec::new(),
            history_folder_id: MenuId::new("history-folder"),
            history_clear_id: MenuId::new("history-clear"),
            autostart: auto,
        })
    }

    pub fn set_hotkey_label(&self, hotkey_label: &str) {
        self.capture.set_text(capture_label(hotkey_label));
        let _ = self.icon.set_tooltip(Some(format!("Frostshot: {hotkey_label}")));
    }

    /// Пересобрать подменю «История» из списка снимков.
    pub fn set_history(&mut self, entries: &[crate::history::Entry], enabled: bool) {
        while self.history.remove_at(0).is_some() {}
        self.history_items.clear();
        if !enabled {
            let _ = self.history.append(&MenuItem::new("Выключена в настройках", false, None));
            return;
        }
        if entries.is_empty() {
            let _ = self.history.append(&MenuItem::new("Пока пусто", false, None));
        }
        for e in entries {
            let icon = std::fs::read(&e.thumb)
                .ok()
                .and_then(|b| Pixmap::decode_png(&b).ok())
                .and_then(|p| tray_icon::menu::Icon::from_rgba(rgba_straight(&p), p.width(), p.height()).ok());
            let item = IconMenuItem::new(&e.label, true, icon, None);
            self.history_items.push((item.id().clone(), e.path.clone()));
            let _ = self.history.append(&item);
        }
        let _ = self.history.append(&PredefinedMenuItem::separator());
        let _ = self.history.append(&MenuItem::with_id(self.history_folder_id.clone(), "Открыть папку истории", true, None));
        let _ = self.history.append(&MenuItem::with_id(self.history_clear_id.clone(), "Очистить историю", !entries.is_empty(), None));
    }

    pub fn set_last_enabled(&self, on: bool) {
        self.last.set_enabled(on);
    }

    pub fn set_autostart(&self, on: bool) {
        self.autostart.set_checked(on);
    }
}
