use tiny_skia::{Pixmap, Rect};
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    icon: TrayIcon,
    capture: MenuItem,
    autostart: CheckMenuItem,
    pub capture_id: MenuId,
    pub folder_id: MenuId,
    pub settings_id: MenuId,
    pub autostart_id: MenuId,
    pub quit_id: MenuId,
}

/// Иконка: синий скруглённый квадрат со снежинкой.
pub fn icon_pixmap(size: u32) -> Pixmap {
    let mut pm = Pixmap::new(size, size).unwrap();
    let s = size as f32;
    crate::draw::fill_rounded(&mut pm, Rect::from_xywh(0.0, 0.0, s, s).unwrap(), s * 0.22, [0x37, 0x8a, 0xdd], 1.0);
    let c = s / 2.0;
    let r = s * 0.34;
    let w = (s / 14.0).max(1.5);
    for i in 0..3 {
        let a = i as f32 * std::f32::consts::PI / 3.0 + std::f32::consts::FRAC_PI_2;
        let (dx, dy) = (a.cos() * r, a.sin() * r);
        crate::draw::line(&mut pm, c - dx, c - dy, c + dx, c + dy, [255, 255, 255], 1.0, w, None);
        for sign in [-1.0f32, 1.0] {
            let (tx, ty) = (c + sign * dx * 0.62, c + sign * dy * 0.62);
            for off in [-0.6f32, 0.6] {
                let b = a + off + if sign < 0.0 { std::f32::consts::PI } else { 0.0 };
                let l = r * 0.33;
                crate::draw::line(&mut pm, tx, ty, tx + b.cos() * l, ty + b.sin() * l, [255, 255, 255], 1.0, w * 0.8, None);
            }
        }
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
        let folder = MenuItem::new("Открыть папку со снимками", true, None);
        let settings = MenuItem::new("Настройки…", true, None);
        let auto = CheckMenuItem::new("Запускать при входе в систему", true, autostart, None);
        let quit = MenuItem::new("Выход", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &capture,
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
            folder_id: folder.id().clone(),
            settings_id: settings.id().clone(),
            autostart_id: auto.id().clone(),
            quit_id: quit.id().clone(),
            capture,
            autostart: auto,
        })
    }

    pub fn set_hotkey_label(&self, hotkey_label: &str) {
        self.capture.set_text(capture_label(hotkey_label));
        let _ = self.icon.set_tooltip(Some(format!("Frostshot: {hotkey_label}")));
    }

    pub fn set_autostart(&self, on: bool) {
        self.autostart.set_checked(on);
    }
}
