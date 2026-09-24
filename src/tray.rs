use tiny_skia::{Pixmap, Rect};
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    icon: TrayIcon,
    capture: MenuItem,
    last: MenuItem,
    autostart: CheckMenuItem,
    pub capture_id: MenuId,
    pub last_id: MenuId,
    pub open_project_id: MenuId,
    pub folder_id: MenuId,
    pub settings_id: MenuId,
    pub autostart_id: MenuId,
    pub quit_id: MenuId,
}

/// Иконка: тёмно-синий градиент, уголки кропа и ледяной кристалл из четырёх граней.
/// Рисуется в сетке 128x128 и масштабируется; в мелких размерах уголки толще.
pub fn icon_pixmap(size: u32) -> Pixmap {
    use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Point, SpreadMode, Transform};
    let mut pm = Pixmap::new(size, size).unwrap();
    let s = size as f32;
    let k = s / 128.0;
    let small = size <= 24;

    // Фон: градиент от яркого синего слева сверху к глубокому снизу справа.
    if let Some(path) = crate::draw::rounded_rect(Rect::from_xywh(0.0, 0.0, s, s).unwrap(), 28.0 * k) {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        paint.shader = LinearGradient::new(
            Point::from_xy(0.0, 0.0),
            Point::from_xy(s, s),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(0x2f, 0x7f, 0xe0, 255)),
                GradientStop::new(1.0, Color::from_rgba8(0x0a, 0x33, 0x7a, 255)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap_or(tiny_skia::Shader::SolidColor(Color::from_rgba8(0x1f, 0x5f, 0xa8, 255)));
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // Уголки кропа.
    let corner = [0xd6, 0xec, 0xff];
    let (m, len) = (22.0, 22.0);
    let w = if small { 12.0 } else { 9.0 } * k;
    let p = |x: f32, y: f32| (x * k, y * k);
    for (cx, cy, dx, dy) in [(m, m, 1.0, 1.0), (128.0 - m, m, -1.0, 1.0), (m, 128.0 - m, 1.0, -1.0), (128.0 - m, 128.0 - m, -1.0, -1.0)] {
        let mut pb = PathBuilder::new();
        let a = p(cx, cy + dy * len);
        let b = p(cx, cy);
        let c = p(cx + dx * len, cy);
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        pb.line_to(c.0, c.1);
        if let Some(path) = pb.finish() {
            crate::draw::stroke_path(&mut pm, &path, corner, 1.0, w, None);
        }
    }

    // Кристалл: ромб, четыре грани разной яркости, как у огранённого льда.
    let (top, right, bottom, left, mid) = (p(64.0, 30.0), p(92.0, 64.0), p(64.0, 98.0), p(36.0, 64.0), p(64.0, 60.0));
    let facets: [([(f32, f32); 3], [u8; 3]); 4] = [
        ([top, left, mid], [0xbf, 0xe3, 0xff]),
        ([top, right, mid], [0xff, 0xff, 0xff]),
        ([left, bottom, mid], [0x6f, 0xb6, 0xf2]),
        ([right, bottom, mid], [0x9c, 0xd0, 0xfa]),
    ];
    for (pts, c) in facets {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        pb.line_to(pts[1].0, pts[1].1);
        pb.line_to(pts[2].0, pts[2].1);
        pb.close();
        if let Some(path) = pb.finish() {
            pm.fill_path(&path, &crate::draw::paint(c, 1.0), FillRule::Winding, Transform::identity(), None);
        }
    }
    // Лёгкий контур, чтобы кристалл не сливался с фоном в мелком размере.
    let mut pb = PathBuilder::new();
    pb.move_to(top.0, top.1);
    pb.line_to(right.0, right.1);
    pb.line_to(bottom.0, bottom.1);
    pb.line_to(left.0, left.1);
    pb.close();
    if let Some(path) = pb.finish() {
        crate::draw::stroke_path(&mut pm, &path, [0xe8, 0xf6, 0xff], 0.9, (1.6 * k).max(0.8), None);
    }
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
        let open_project = MenuItem::new("Открыть проект…", true, None);
        let folder = MenuItem::new("Открыть папку со снимками", true, None);
        let settings = MenuItem::new("Настройки…", true, None);
        let auto = CheckMenuItem::new("Запускать при входе в систему", true, autostart, None);
        let quit = MenuItem::new("Выход", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &capture,
            &last,
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
            autostart: auto,
        })
    }

    pub fn set_hotkey_label(&self, hotkey_label: &str) {
        self.capture.set_text(capture_label(hotkey_label));
        let _ = self.icon.set_tooltip(Some(format!("Frostshot: {hotkey_label}")));
    }

    pub fn set_last_enabled(&self, on: bool) {
        self.last.set_enabled(on);
    }

    pub fn set_autostart(&self, on: bool) {
        self.autostart.set_checked(on);
    }
}
