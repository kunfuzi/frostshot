use tiny_skia::{Pixmap, Rect};
use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    _icon: TrayIcon,
    pub capture_id: MenuId,
    pub folder_id: MenuId,
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

pub fn build(hotkey_label: &str) -> Result<Tray, String> {
    let capture = MenuItem::new(format!("Сделать скриншот ({hotkey_label})"), true, None);
    let folder = MenuItem::new("Открыть папку со снимками", true, None);
    let quit = MenuItem::new("Выход", true, None);
    let menu = Menu::new();
    menu.append_items(&[&capture, &folder, &PredefinedMenuItem::separator(), &quit])
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
        _icon: tray,
        capture_id: capture.id().clone(),
        folder_id: folder.id().clone(),
        quit_id: quit.id().clone(),
    })
}
