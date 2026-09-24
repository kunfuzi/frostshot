//! `frostshot --selftest <dir>`: сценарий без окон. Реальный захват, программный ввод,
//! кадры и результаты в PNG, проверки в stdout.

use crate::session::{Action, Mods, Session};
use crate::{capture, draw, output};
use std::path::Path;
use std::sync::Arc;
use winit::keyboard::{KeyCode, NamedKey};

struct Check {
    fails: u32,
}

impl Check {
    fn ok(&mut self, name: &str, cond: bool) {
        println!("{} {name}", if cond { "PASS" } else { "FAIL" });
        if !cond {
            self.fails += 1;
        }
    }
}

fn drag(s: &mut Session, mon: usize, a: (f32, f32), b: (f32, f32)) {
    s.on_left_press(mon, a.0, a.1);
    for i in 1..=10 {
        let t = i as f32 / 10.0;
        s.on_move(mon, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    }
    s.on_left_release(mon, b.0, b.1);
}

/// Проект с подменённым списком фигур (для проверки разбора недоверенных файлов).
fn craft_evil(bytes: &[u8], shape_json: &str) -> Option<Vec<u8>> {
    let rest = &bytes[10..];
    let len = u32::from_le_bytes(rest[..4].try_into().ok()?) as usize;
    let mut v: serde_json::Value = serde_json::from_slice(&rest[4..4 + len]).ok()?;
    v["shapes"] = serde_json::Value::Array(vec![serde_json::from_str(shape_json).ok()?]);
    let json = serde_json::to_vec(&v).ok()?;
    let mut out = bytes[..10].to_vec();
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&rest[4 + len..]);
    Some(out)
}

fn alpha_at(img: &tiny_skia::Pixmap, x: u32, y: u32) -> u8 {
    img.data()[((y * img.width() + x) * 4 + 3) as usize]
}

pub fn run(dir: &Path) -> i32 {
    std::fs::create_dir_all(dir).ok();
    let mut c = Check { fails: 0 };
    let t0 = std::time::Instant::now();
    let shots = match capture::capture_all() {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL capture: {e}");
            return 1;
        }
    };
    println!("INFO captured {} monitors in {:?}", shots.len(), t0.elapsed());
    for (i, s) in shots.iter().enumerate() {
        println!("INFO monitor {i}: at {},{} size {}x{}", s.x, s.y, s.width(), s.height());
        output::save_png(&s.pixmap, &dir.join(format!("shot_{i}.png"))).ok();
    }
    let sizes: Vec<(u32, u32)> = shots.iter().map(|s| (s.width(), s.height())).collect();
    let font = draw::load_font().map(Arc::new);
    c.ok("system font loaded", font.is_some());
    let n = shots.len();
    let mut s = Session::new(shots, vec![1.0; n], 0.5, 0xE24B4A, 4.0, font);

    // 1. Клик без протяжки = весь монитор.
    s.on_left_press(0, 300.0, 300.0);
    s.on_left_release(0, 301.0, 300.0);
    let img = s.result().expect("result");
    c.ok("click selects whole monitor", (img.width(), img.height()) == sizes[0]);

    // 2. Прямоугольник, затем добавление (Shift) и вычитание (Alt).
    drag(&mut s, 0, (100.0, 100.0), (500.0, 400.0));
    let img = s.result().unwrap();
    c.ok("rect selection 400x300", (img.width(), img.height()) == (400, 300));
    s.mods = Mods { shift: true, ..Default::default() };
    drag(&mut s, 0, (450.0, 350.0), (700.0, 600.0));
    s.mods = Mods { alt: true, ..Default::default() };
    drag(&mut s, 0, (150.0, 150.0), (250.0, 250.0));
    s.mods = Mods::default();
    let img = s.result().unwrap();
    c.ok("union bbox 600x500", (img.width(), img.height()) == (600, 500));
    c.ok("subtracted area transparent", alpha_at(&img, 100, 100) == 0);
    c.ok("kept area opaque", alpha_at(&img, 20, 20) == 255);
    c.ok("outside union transparent", alpha_at(&img, 550, 50) == 0);
    c.ok("added rect opaque", alpha_at(&img, 500, 400) == 255);

    // 3. Разметка всеми инструментами.
    for (key, a, b) in [
        (KeyCode::Digit4, (200.0, 350.0), (400.0, 200.0)),
        (KeyCode::Digit5, (300.0, 120.0), (480.0, 180.0)),
        (KeyCode::Digit3, (110.0, 380.0), (300.0, 380.0)),
        (KeyCode::Digit1, (120.0, 300.0), (180.0, 330.0)),
        (KeyCode::Digit2, (320.0, 300.0), (480.0, 300.0)),
        (KeyCode::Digit7, (500.0, 450.0), (650.0, 550.0)),
    ] {
        s.on_key(Some(key), None, None);
        drag(&mut s, 0, a, b);
    }
    s.on_key(Some(KeyCode::Digit6), None, None);
    s.on_left_press(0, 130.0, 270.0);
    s.on_left_release(0, 130.0, 270.0);
    s.on_key(None, None, Some("Привет, Frostshot"));
    s.on_key(None, Some(NamedKey::Escape), None);
    s.on_move(0, 505.0, 110.0);
    let frame = s.render(0).clone();
    output::save_png(&frame, &dir.join("frame_annotated.png")).ok();
    let img = s.result().unwrap();
    output::save_png(&img, &dir.join("result_annotated.png")).ok();
    // Стрелка красная рядом с остриём (400,200) -> в координатах результата (300,100).
    let px = |img: &tiny_skia::Pixmap, x: u32, y: u32| {
        let i = ((y * img.width() + x) * 4) as usize;
        [img.data()[i], img.data()[i + 1], img.data()[i + 2]]
    };
    let p = px(&img, 296, 104);
    c.ok("arrow head drawn in red", p[0] > 180 && p[1] < 110 && p[2] < 110);

    // Undo убирает текст.
    let before = s.result().unwrap();
    s.mods = Mods { ctrl: true, ..Default::default() };
    s.on_key(Some(KeyCode::KeyZ), None, None);
    s.mods = Mods::default();
    let after = s.result().unwrap();
    c.ok("undo changes image", before.data() != after.data());

    // AltGr (Ctrl+Alt) вводит символ в текст, а не срабатывает как шорткат.
    s.on_key(Some(KeyCode::Digit6), None, None);
    s.on_left_press(0, 400.0, 300.0);
    s.on_left_release(0, 400.0, 300.0);
    let before = s.result().unwrap();
    s.on_key(Some(KeyCode::Digit6), None, None);
    s.on_left_press(0, 400.0, 300.0);
    s.on_left_release(0, 400.0, 300.0);
    s.mods = Mods { ctrl: true, alt: true, ..Default::default() };
    s.on_key(Some(KeyCode::KeyE), None, Some("€"));
    s.mods = Mods::default();
    s.on_key(None, Some(NamedKey::Escape), None);
    let after = s.result().unwrap();
    c.ok("altgr char typed into text", before.data() != after.data());
    s.mods = Mods { ctrl: true, ..Default::default() };
    s.on_key(Some(KeyCode::KeyZ), None, None);
    s.mods = Mods::default();

    // Новые инструменты: закрашенный прямоугольник, эллипс, счётчик; повтор.
    let base_img = s.result().unwrap();
    s.on_key(Some(KeyCode::Digit8), None, None);
    drag(&mut s, 0, (120.0, 120.0), (160.0, 160.0));
    let img = s.result().unwrap();
    let q = px(&img, 40, 40);
    c.ok("filled rect is solid red", q[0] > 200 && q[1] < 90 && q[2] < 90);
    s.on_key(Some(KeyCode::Digit9), None, None);
    drag(&mut s, 0, (300.0, 200.0), (420.0, 260.0));
    s.on_key(Some(KeyCode::Digit0), None, None);
    for (x, y) in [(200.0, 300.0), (240.0, 300.0), (280.0, 300.0)] {
        s.on_left_press(0, x, y);
        s.on_left_release(0, x, y);
    }
    let frame = s.render(0).clone();
    output::save_png(&frame, &dir.join("frame_new_tools.png")).ok();
    let with_all = s.result().unwrap();
    s.mods = Mods { ctrl: true, ..Default::default() };
    s.on_key(Some(KeyCode::KeyZ), None, None); // убрать счётчик 3
    s.mods = Mods::default();
    s.on_left_press(0, 320.0, 300.0);
    s.on_left_release(0, 320.0, 300.0); // снова 3, повтор очищен
    s.mods = Mods { ctrl: true, shift: true, ..Default::default() };
    s.on_key(Some(KeyCode::KeyZ), None, None); // повторять нечего
    s.mods = Mods { ctrl: true, ..Default::default() };
    for _ in 0..5 {
        s.on_key(Some(KeyCode::KeyZ), None, None);
    }
    let undone = s.result().unwrap();
    c.ok("undo all new shapes", undone.data() == base_img.data());
    for _ in 0..5 {
        s.on_key(Some(KeyCode::KeyY), None, None);
    }
    s.mods = Mods::default();
    let redone = s.result().unwrap();
    c.ok("redo restores shapes", redone.data() != undone.data());
    let _ = with_all;
    c.ok("P pins selection", s.on_key(Some(KeyCode::KeyP), None, None) == Action::Pin);
    c.ok("selection origin known", s.selection_origin().is_some());
    s.mods = Mods { ctrl: true, ..Default::default() };
    for _ in 0..5 {
        s.on_key(Some(KeyCode::KeyZ), None, None);
    }
    s.mods = Mods::default();

    // 4. Горячие клавиши действий.
    s.mods = Mods { ctrl: true, ..Default::default() };
    c.ok("ctrl+c -> copy", s.on_key(Some(KeyCode::KeyC), None, None) == Action::Copy);
    c.ok("ctrl+s -> save", s.on_key(Some(KeyCode::KeyS), None, None) == Action::Save);
    s.mods = Mods { ctrl: true, shift: true, ..Default::default() };
    c.ok("ctrl+shift+s -> quick save", s.on_key(Some(KeyCode::KeyS), None, None) == Action::QuickSave);
    s.mods = Mods::default();

    // 5. Перемещение выделения (рамка).
    s.on_key(Some(KeyCode::KeyV), None, None);
    s.on_right_press();
    drag(&mut s, 0, (100.0, 100.0), (500.0, 400.0));
    drag(&mut s, 0, (300.0, 250.0), (350.0, 270.0));
    let img = s.result().unwrap();
    c.ok("move keeps size", (img.width(), img.height()) == (400, 300));
    // Ресайз за правый нижний угол (550,420) -> (600,500).
    drag(&mut s, 0, (550.0, 420.0), (600.0, 500.0));
    let img = s.result().unwrap();
    c.ok("resize by corner 450x380", (img.width(), img.height()) == (450, 380));

    // 6. Лассо на последнем мониторе.
    let last = n - 1;
    s.on_key(Some(KeyCode::KeyL), None, None);
    let (cx, cy, r) = (400.0f32, 400.0f32, 150.0f32);
    s.on_left_press(last, cx + r, cy);
    for i in 1..=64 {
        let a = i as f32 / 64.0 * std::f32::consts::TAU;
        s.on_move(last, cx + r * a.cos(), cy + r * a.sin());
    }
    s.on_left_release(last, cx + r, cy);
    let img = s.result().unwrap();
    output::save_png(&img, &dir.join("result_lasso.png")).ok();
    c.ok("lasso bbox ~300x300", (img.width() as i32 - 300).abs() <= 2 && (img.height() as i32 - 300).abs() <= 2);
    c.ok("lasso corner transparent", alpha_at(&img, 5, 5) == 0);
    c.ok("lasso center opaque", alpha_at(&img, 150, 150) == 255);
    let frame = s.render(last).clone();
    output::save_png(&frame, &dir.join("frame_lasso.png")).ok();

    // Проект .frost: сохранение и загрузка дают тот же результат.
    let before = s.result().unwrap();
    let bytes = s.to_project().expect("to_project");
    output::save_png(&before, &dir.join("project_before.png")).ok();
    std::fs::write(dir.join("roundtrip.frost"), &bytes).ok();
    match Session::from_project(&bytes, 0, 0, 1.0, 0.5, draw::load_font().map(Arc::new)) {
        Ok(mut p) => {
            let after = p.result().unwrap();
            c.ok("project roundtrip same image", before.data() == after.data());
            let frame = p.render(0).clone();
            output::save_png(&frame, &dir.join("frame_project.png")).ok();
        }
        Err(e) => c.ok(&format!("project roundtrip ({e})"), false),
    }
    // Вредоносный проект: отрицательная толщина пикселизации и NaN не должны вешать программу.
    if let Some(evil) = craft_evil(&bytes, r#"{"kind":{"Pixelate":[[10.0,10.0],[200.0,200.0]]},"color":[0,0,0],"width":-4.0}"#) {
        match Session::from_project(&evil, 0, 0, 1.0, 0.5, None) {
            Ok(mut p) => {
                let t = std::time::Instant::now();
                p.render(0);
                c.ok("evil width clamped, render finishes", t.elapsed().as_secs() < 2);
            }
            Err(e) => c.ok(&format!("evil width rejected ({e})"), true),
        }
    }
    if let Some(evil) = craft_evil(&bytes, r#"{"kind":{"Line":[[1e39,0.0],[5.0,5.0]]},"color":[0,0,0],"width":3.0}"#) {
        c.ok("non-finite coordinates rejected", Session::from_project(&evil, 0, 0, 1.0, 0.5, None).is_err());
    }
    c.ok("garbage is not a project", Session::from_project(b"PNG junk", 0, 0, 1.0, 0.5, None).is_err());
    let mut cut = bytes.clone();
    cut.truncate(bytes.len() - 100);
    c.ok("truncated project rejected", Session::from_project(&cut, 0, 0, 1.0, 0.5, None).is_err());

    // Сон и пробуждение сессии сохраняют разметку и выделение.
    s.hibernate();
    s.wake();
    c.ok("hibernate/wake keeps result", s.result().is_some_and(|r| r.data() == before.data()));
    s.render(last);

    // Меню сохранения: кнопка открывает меню, пункт даёт действие.
    s.on_key(Some(KeyCode::KeyV), None, None);

    // 7. Правый клик сбрасывает, второй закрывает.
    c.ok("right click resets selection", s.on_right_press() == Action::None && s.result().is_none());
    c.ok("second right click closes", s.on_right_press() == Action::Close);

    // 8. Скорость отрисовки кадра.
    s.on_move(0, 200.0, 200.0);
    let t = std::time::Instant::now();
    for _ in 0..10 {
        s.render(0);
    }
    let per = t.elapsed() / 10;
    println!("INFO render frame {}x{}: {:?}", sizes[0].0, sizes[0].1, per);
    let frame = s.render(0).clone();
    output::save_png(&frame, &dir.join("frame_idle.png")).ok();

    println!("{} failures", c.fails);
    if c.fails == 0 { 0 } else { 1 }
}
