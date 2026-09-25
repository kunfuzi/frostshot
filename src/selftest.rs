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

/// Картинка вместо снимка экрана: тёмный фон, светлые «окна» и полосы «текста».
fn synthetic_screen(w: u32, h: u32, seed: u8) -> tiny_skia::Pixmap {
    let mut pm = tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(24 + seed * 6, 28, 36, 255));
    for i in 0..12u32 {
        let (x, y) = ((i * 157 + seed as u32 * 40) % (w - 300), (i * 89) % (h - 200));
        if let Some(r) = tiny_skia::Rect::from_xywh(x as f32, y as f32, 280.0, 180.0) {
            draw::fill_rect(&mut pm, r, [40 + (i * 13 % 90) as u8, 60 + (i * 7 % 80) as u8, 90 + (i * 11 % 100) as u8], 1.0, None);
        }
        for j in 0..5u32 {
            if let Some(r) = tiny_skia::Rect::from_xywh(x as f32 + 14.0, y as f32 + 20.0 + j as f32 * 26.0, 180.0 + (j * 17 % 60) as f32, 8.0) {
                draw::fill_rect(&mut pm, r, [200, 205, 215], 1.0, None);
            }
        }
    }
    pm
}

fn alpha_at(img: &tiny_skia::Pixmap, x: u32, y: u32) -> u8 {
    img.data()[((y * img.width() + x) * 4 + 3) as usize]
}

pub fn run(dir: &Path) -> i32 {
    std::fs::create_dir_all(dir).ok();
    let mut c = Check { fails: 0 };
    let t0 = std::time::Instant::now();
    // Экран недоступен (заблокирован, сеанс отключён): два нарисованных монитора,
    // проверкам выделения, фигур и панелей настоящий снимок не нужен.
    let shots = match capture::capture_all() {
        Ok(s) if !s.is_empty() => s,
        r => {
            println!("INFO capture unavailable ({}), using synthetic monitors", r.err().unwrap_or_else(|| "no monitors".into()));
            (0..2).map(|i| capture::MonitorShot { x: i * 1920, y: 0, pixmap: synthetic_screen(1920, 1080, i as u8) }).collect()
        }
    };
    println!("INFO captured {} monitors in {:?}", shots.len(), t0.elapsed());
    for (i, s) in shots.iter().enumerate() {
        println!("INFO monitor {i}: at {},{} size {}x{}", s.x, s.y, s.width(), s.height());
        output::save_png(&s.pixmap, &dir.join(format!("shot_{i}.png"))).ok();
    }
    for sh in &shots {
        let mm = crate::platform::monitor_mm_per_px(sh.x + sh.width() as i32 / 2, sh.y + sh.height() as i32 / 2, sh.width());
        println!("INFO monitor at {},{}: mm per px {:?} (width {:?} mm)", sh.x, sh.y, mm, mm.map(|k| k * sh.width() as f32));
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
    // Счётчик с выноской, Shift: нажатие на цель, отпускание в стороне.
    s.mods = Mods { shift: true, ..Default::default() };
    drag(&mut s, 0, (150.0, 380.0), (260.0, 380.0));
    s.mods = Mods::default();
    let img = s.result().unwrap();
    let q = px(&img, 105, 280);
    c.ok("counter callout wedge drawn", q[0] > 200 && q[1] < 90 && q[2] < 90);
    // Без Shift: кружок в точке нажатия, клин к месту отпускания.
    drag(&mut s, 0, (650.0, 580.0), (550.0, 580.0));
    let img = s.result().unwrap();
    let q = px(&img, 550, 480); // центр кружка (650,580) в координатах результата
    let t = px(&img, 500, 480); // середина клина (600,580)
    c.ok("label-first counter at press point", q[0] > 200 && q[1] < 90);
    c.ok("label-first wedge toward target", t[0] > 200 && t[1] < 90);
    let frame = s.render(0).clone();
    output::save_png(&frame, &dir.join("frame_new_tools.png")).ok();
    let with_all = s.result().unwrap();

    // SVG: все фигуры векторами, фигурное выделение маской. Добавим текст и линейку.
    s.on_key(Some(KeyCode::Digit6), None, None);
    s.on_left_press(0, 420.0, 230.0);
    s.on_left_release(0, 420.0, 230.0);
    s.on_key(Some(KeyCode::KeyA), None, Some("Ab <&> 12"));
    s.on_key(None, Some(NamedKey::Escape), None);
    s.on_key(Some(KeyCode::KeyR), None, None);
    drag(&mut s, 0, (560.0, 380.0), (680.0, 540.0));
    let svg_ref = s.result().unwrap();
    match s.to_svg(false) {
        Ok(svg) => {
            std::fs::write(dir.join("export.svg"), &svg).ok();
            output::save_png(&svg_ref, &dir.join("export_reference.png")).ok();
            let size = format!(r#"width="{}" height="{}""#, svg_ref.width(), svg_ref.height());
            c.ok("svg size equals png result", svg.contains(&size));
            c.ok("svg uses selection mask", svg.contains(r#"mask="url(#selection)""#));
            c.ok("svg text escaped", svg.contains("Ab &lt;&amp;&gt; 12"));
            c.ok("svg ruler label", svg.contains(" px ("));
            for tag in ["<polyline", "<line", "<rect", "<ellipse", "<circle", "<polygon", "<text", "<image"] {
                c.ok(&format!("svg has {tag}"), svg.contains(tag));
            }
        }
        Err(e) => c.ok(&format!("svg export: {e}"), false),
    }
    // Убрать текст и линейку: дальше проверки ждут прежний набор фигур.
    s.mods = Mods { ctrl: true, ..Default::default() };
    s.on_key(Some(KeyCode::KeyZ), None, None);
    s.on_key(Some(KeyCode::KeyZ), None, None);
    s.mods = Mods::default();
    c.ok("svg extras undone", s.result().unwrap().data() == with_all.data());
    s.mods = Mods { ctrl: true, ..Default::default() };
    s.on_key(Some(KeyCode::KeyZ), None, None); // убрать счётчик 3
    s.mods = Mods::default();
    s.on_left_press(0, 320.0, 300.0);
    s.on_left_release(0, 320.0, 300.0); // снова 3, повтор очищен
    s.mods = Mods { ctrl: true, shift: true, ..Default::default() };
    s.on_key(Some(KeyCode::KeyZ), None, None); // повторять нечего
    s.mods = Mods { ctrl: true, ..Default::default() };
    for _ in 0..7 {
        s.on_key(Some(KeyCode::KeyZ), None, None);
    }
    let undone = s.result().unwrap();
    c.ok("undo all new shapes", undone.data() == base_img.data());
    for _ in 0..7 {
        s.on_key(Some(KeyCode::KeyY), None, None);
    }
    s.mods = Mods::default();
    let redone = s.result().unwrap();
    c.ok("redo restores shapes", redone.data() != undone.data());
    let _ = with_all;
    c.ok("P pins selection", s.on_key(Some(KeyCode::KeyP), None, None) == Action::Pin);
    c.ok("selection origin known", s.selection_origin().is_some());
    s.mods = Mods { ctrl: true, ..Default::default() };
    for _ in 0..7 {
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
    s.on_key(Some(KeyCode::KeyM), None, None);
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
        // Битая фигура пропадает, проект открывается (или файл целиком отклонён).
        let ok = Session::from_project(&evil, 0, 0, 1.0, 0.5, None).map_or(true, |p| p.shapes().is_empty());
        c.ok("non-finite coordinates: shape dropped", ok);
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
    s.on_key(Some(KeyCode::KeyM), None, None);

    // Образец толщины после прокрутки колеса (для разных инструментов).
    let mut tiles: Vec<tiny_skia::Pixmap> = Vec::new();
    s.on_key(Some(KeyCode::KeyM), None, None);
    s.on_right_press();
    drag(&mut s, 0, (100.0, 100.0), (700.0, 500.0));
    for key in [KeyCode::Digit4, KeyCode::Digit2, KeyCode::Digit6, KeyCode::Digit0] {
        s.on_key(Some(key), None, None);
        s.on_move(0, 250.0, 330.0);
        s.on_wheel(1.0);
        s.on_wheel(1.0);
        let f = s.render(0).clone();
        if let Some(t) = f.clone_rect(tiny_skia::IntRect::from_xywh(240, 130, 220, 210).unwrap()) {
            tiles.push(t);
        }
    }
    c.ok("width hint scheduled", s.width_hint_deadline().is_some());
    std::thread::sleep(crate::session::WIDTH_HINT + std::time::Duration::from_millis(100));
    s.expire_width_hint();
    c.ok("width hint expires", s.width_hint_deadline().is_none());
    if let Some(mut sheet) = tiny_skia::Pixmap::new(220 * tiles.len() as u32, 210) {
        for (i, t) in tiles.iter().enumerate() {
            sheet.draw_pixmap(220 * i as i32, 0, t.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }
        output::save_png(&sheet, &dir.join("width_hints.png")).ok();
    }
    for _ in 0..8 {
        s.on_wheel(-1.0);
    }
    s.expire_width_hint();

    // Масштаб: Ctrl + колесо приближает к курсору, выделение в увеличенном виде точное.
    s.on_key(Some(KeyCode::KeyM), None, None);
    s.on_right_press();
    for _ in 0..3 {
        s.on_zoom(0, 1.0, 400.0, 300.0);
    }
    let z = 1.25f32.powi(3);
    drag(&mut s, 0, (200.0, 200.0), (400.0, 400.0));
    let img = s.result().unwrap();
    let want = (200.0 / z).round() as i32;
    c.ok(&format!("zoomed selection {}x{} ~ {want}", img.width(), img.height()), (img.width() as i32 - want).abs() <= 1 && (img.height() as i32 - want).abs() <= 1);
    s.on_move(0, 300.0, 300.0);
    let frame = s.render(0).clone();
    output::save_png(&frame, &dir.join("frame_zoom.png")).ok();
    s.mods = Mods { ctrl: true, ..Default::default() };
    s.on_key(Some(KeyCode::Digit0), None, None);
    s.mods = Mods::default();
    s.on_right_press();
    drag(&mut s, 0, (200.0, 200.0), (400.0, 400.0));
    c.ok("ctrl+0 resets zoom", s.result().is_some_and(|i| i.width() == 200));

    // Линейка: подпись длины и отрисовка.
    c.ok("ruler label", crate::shapes::ruler_label((0.0, 0.0), (200.0, 150.0), None) == "250 px (200 × 150)");
    c.ok("ruler label straight", crate::shapes::ruler_label((10.0, 5.0), (130.0, 5.0), Some(0.25)) == "120 px · 30,0 мм");

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

    // Панель инструментов: каждый инструмент ровно один раз, кнопки не перекрываются.
    let args = |bbox: tiny_skia::Rect, mw: f32, mh: f32, cols: usize, style_pop: Option<Option<crate::shapes::Tool>>, save_menu: bool, tools_at: Option<(f32, f32)>| {
        crate::ui::layout(&crate::ui::LayoutArgs { bbox, mw, mh, s: 1.0, save_menu, style_pop, tools_at, cols, font: None })
    };
    let l = args(tiny_skia::Rect::from_xywh(100.0, 100.0, 400.0, 300.0).unwrap(), 1920.0, 1080.0, 2, None, false, None);
    for t in crate::shapes::Tool::ALL {
        let n = l.buttons.iter().filter(|(b, _)| *b == crate::ui::Btn::Tool(t)).count();
        c.ok(&format!("toolbar has {t:?} once"), n == 1);
    }
    let overlap = l.buttons.iter().enumerate().any(|(i, (_, a))| {
        l.buttons[i + 1..].iter().any(|(_, b)| a.left() < b.right() && b.left() < a.right() && a.top() < b.bottom() && b.top() < a.bottom())
    });
    c.ok("toolbar buttons do not overlap", !overlap);
    // Низкий монитор: группы уходят в соседний столбец и помещаются по высоте.
    let low = args(tiny_skia::Rect::from_xywh(50.0, 50.0, 200.0, 100.0).unwrap(), 800.0, 300.0, 2, None, false, None);
    c.ok("toolbar fits low monitor", low.panels[0].bottom() <= 300.0);
    let low1 = args(tiny_skia::Rect::from_xywh(50.0, 50.0, 200.0, 100.0).unwrap(), 800.0, 300.0, 1, None, false, None);
    c.ok("one-column toolbar fits low monitor", low1.panels[0].bottom() <= 300.0);
    let one = args(tiny_skia::Rect::from_xywh(100.0, 100.0, 400.0, 300.0).unwrap(), 1920.0, 1080.0, 1, None, false, None);
    c.ok("one column is narrower", one.panels[0].width() < l.panels[0].width());
    // Кнопка стиля в две колонки: плитка на обе колонки.
    let style_w = |l: &crate::ui::Layout| l.buttons.iter().find(|(b, _)| *b == crate::ui::Btn::Style).map(|(_, r)| r.width());
    c.ok("style tile spans two columns", style_w(&l) == Some(64.0) && style_w(&one) == Some(32.0));
    // Окно стиля: 8 цветов, у стрелки 5 размеров, у закрашенного прямоугольника размеров нет.
    let count = |l: &crate::ui::Layout, f: fn(&crate::ui::Btn) -> bool| l.buttons.iter().filter(|(b, _)| f(b)).count();
    let pop = args(tiny_skia::Rect::from_xywh(100.0, 100.0, 400.0, 300.0).unwrap(), 1920.0, 1080.0, 2, Some(Some(crate::shapes::Tool::Arrow)), false, None);
    let pop_fr = args(tiny_skia::Rect::from_xywh(100.0, 100.0, 400.0, 300.0).unwrap(), 1920.0, 1080.0, 2, Some(None), false, None);
    c.ok(
        "style popover: colors and presets",
        count(&pop, |b| matches!(b, crate::ui::Btn::Swatch(_))) == 8
            && count(&pop, |b| matches!(b, crate::ui::Btn::Preset(_))) == 5
            && count(&pop_fr, |b| matches!(b, crate::ui::Btn::Preset(_))) == 0,
    );
    c.ok("style popover opens away from the selection", pop.panels.last().is_some_and(|p| p.left() >= pop.panels[0].right()));
    // Действия: главная «Копировать» шире обычной кнопки.
    let copy_w = l.buttons.iter().find(|(b, _)| *b == crate::ui::Btn::Copy).map(|(_, r)| r.width()).unwrap_or(0.0);
    c.ok("copy button carries a label", copy_w > 64.0);

    ocr_check(&mut c, dir);
    history_check(&mut c);
    edit_check(&mut c, dir, draw::load_font().map(Arc::new));
    fixes_check(&mut c, draw::load_font().map(Arc::new));
    fixes2_check(&mut c, draw::load_font().map(Arc::new));
    panels_check(&mut c, dir, draw::load_font().map(Arc::new));

    println!("{} failures", c.fails);
    if c.fails == 0 { 0 } else { 1 }
}

/// Сквозная проверка: картинка с личными данными -> распознавание Windows -> поиск -> скрытие.
fn ocr_check(c: &mut Check, dir: &Path) {
    let Some(font) = draw::load_font() else { return };
    let (w, h) = (1100u32, 320u32);
    let mut pm = tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(tiny_skia::Color::WHITE);
    let lines = [
        "Почта: ivan.petrov@example.com",
        "Телефон: +7 912 345-67-89",
        "Карта: 4276 3801 2345 6787",
        "Пароль: Qwerty2026 и токен ghp_A1b2C3d4E5f6G7h8I9j0KLMN",
        "Твой номер 4257 1111 2255 6888 4555 теперь есть в юнит-тестах",
    ];
    for (i, t) in lines.iter().enumerate() {
        draw::draw_text(&mut pm, &font, t, 20.0, 20.0 + i as f32 * 56.0, 28.0, [0x11, 0x11, 0x11], 1.0, None);
    }
    output::save_png(&pm, &dir.join("ocr_input.png")).ok();
    let t0 = std::time::Instant::now();
    match crate::platform::ocr_recognize(&pm) {
        Ok(ls) => {
            println!("INFO ocr {:?}: {:?}", t0.elapsed(), crate::ocr::text_of(&ls));
            let found = crate::ocr::find_sensitive(&ls, 3.0);
            println!("INFO {}", crate::ocr::summary(&found));
            let kinds: Vec<_> = found.iter().map(|(k, _)| *k).collect();
            use crate::ocr::Kind;
            for k in [Kind::Email, Kind::Phone, Kind::Card, Kind::Secret, Kind::Number] {
                c.ok(&format!("ocr finds {}", k.label()), kinds.contains(&k));
            }
            // Через сессию: скрытие меняет пиксели на месте почты.
            let shot = crate::capture::MonitorShot { x: 0, y: 0, pixmap: pm.clone() };
            let mut s = Session::new(vec![shot], vec![1.0], 0.5, 0xE24B4A, 4.0, Some(Arc::new(font)));
            s.on_left_press(0, 5.0, 5.0);
            c.ok("selection done reported", s.on_left_release(0, 6.0, 5.0) == Action::SelectionDone);
            let before = s.result().unwrap();
            let rects: Vec<_> = found.iter().map(|(_, r)| *r).collect();
            let added = s.apply_hide(&rects);
            c.ok("auto-hide adds areas", added == rects.len());
            c.ok("auto-hide does not duplicate", s.apply_hide(&rects) == 0);
            let after = s.result().unwrap();
            output::save_png(&after, &dir.join("ocr_hidden.png")).ok();
            c.ok("auto-hide changes the image", before.data() != after.data());
        }
        Err(e) => println!("INFO ocr unavailable: {e}"),
    }
}

/// История: запись проектов, список (новые первыми), лимит количества, очистка.
/// Debug-сборка пишет в отдельную папку history/dev, настоящая история не трогается.
fn history_check(c: &mut Check) {
    use crate::history;
    history::clear();
    let mut img = tiny_skia::Pixmap::new(300, 200).unwrap();
    img.fill(tiny_skia::Color::from_rgba8(40, 120, 220, 255));
    let shot = crate::capture::MonitorShot { x: 0, y: 0, pixmap: img.clone() };
    let mut s = Session::new(vec![shot], vec![1.0], 0.5, 0xE24B4A, 4.0, None);
    s.on_left_press(0, 10.0, 10.0);
    s.on_left_release(0, 11.0, 10.0);
    for _ in 0..3 {
        let (h, shot) = s.project_parts().unwrap();
        let bytes = crate::project::build(h, &shot).unwrap();
        c.ok("history save", history::save(&bytes, &img).is_ok());
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let list = history::list();
    c.ok("history lists 3", list.len() == 3);
    c.ok("history newest first", list.windows(2).all(|w| w[0].modified >= w[1].modified));
    c.ok("history thumbnail", list.iter().all(|e| e.thumb.exists()));
    c.ok("history label has size", list[0].label.contains("300×200"));
    c.ok("history entry opens", Session::from_project(&std::fs::read(&list[0].path).unwrap(), 0, 0, 1.0, 0.5, None).is_ok());
    // Панель истории: полный кадр после анимации и середина анимации.
    for (i, col) in [(0u8, [220u8, 80, 60]), (1, [60, 180, 90]), (2, [200, 160, 40])] {
        let mut im = tiny_skia::Pixmap::new(480 + i as u32 * 120, 300).unwrap();
        im.fill(tiny_skia::Color::from_rgba8(col[0], col[1], col[2], 255));
        let (h, shot) = s.project_parts().unwrap();
        let _ = history::save(&crate::project::build(h, &shot).unwrap(), &im);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let font = draw::load_font();
    if let Some(p) = crate::history_popup::render_preview(&history::list(), 1.25, 2.0, font.as_ref(), true) {
        output::save_png(&p, &std::path::Path::new(&std::env::temp_dir()).join("frostshot_history_popup.png")).ok();
        c.ok("history popup renders", p.width() > 0);
    }
    if let Some(p) = crate::history_popup::render_preview(&history::list(), 1.25, 0.12, font.as_ref(), false) {
        output::save_png(&p, &std::path::Path::new(&std::env::temp_dir()).join("frostshot_history_popup_anim.png")).ok();
    }
    history::prune(2, 7);
    c.ok("history prune to 2", history::list().len() == 2);
    history::clear();
    c.ok("history clear", history::list().is_empty());
}

/// Правка готовых фигур в режиме выделения (V): выбор, перенос, ручки, удаление,
/// отмена, правка текста; новая область не сбрасывает разметку.
fn edit_check(c: &mut Check, dir: &Path, font: Option<Arc<ab_glyph::FontVec>>) {
    use crate::shapes::Kind;
    let mut img = tiny_skia::Pixmap::new(1000, 800).unwrap();
    img.fill(tiny_skia::Color::from_rgba8(60, 90, 140, 255));
    let shot = crate::capture::MonitorShot { x: 0, y: 0, pixmap: img };
    let mut s = Session::new(vec![shot], vec![1.0], 0.5, 0xE24B4A, 4.0, font);
    let click = |s: &mut Session, x: f32, y: f32| {
        s.on_left_press(0, x, y);
        s.on_left_release(0, x, y);
    };
    let ctrl_z = |s: &mut Session, shift: bool| {
        s.mods = Mods { ctrl: true, shift, ..Default::default() };
        s.on_key(Some(KeyCode::KeyZ), None, None);
        s.mods = Mods::default();
    };
    drag(&mut s, 0, (150.0, 150.0), (650.0, 450.0));
    s.on_key(Some(KeyCode::Digit4), None, None);
    drag(&mut s, 0, (200.0, 200.0), (400.0, 300.0));
    s.on_key(Some(KeyCode::Digit5), None, None);
    drag(&mut s, 0, (450.0, 180.0), (600.0, 260.0));
    s.on_key(Some(KeyCode::Digit7), None, None);
    drag(&mut s, 0, (200.0, 330.0), (300.0, 420.0));
    s.on_key(Some(KeyCode::Digit6), None, None);
    click(&mut s, 480.0, 380.0);
    s.on_key(Some(KeyCode::KeyH), None, Some("Hi"));
    s.on_key(None, Some(NamedKey::Escape), None);
    c.ok("edit: 4 shapes drawn", s.shapes().len() == 4);
    let text_at = match s.shapes()[3].kind {
        Kind::Text { at, .. } => at,
        _ => (0.0, 0.0),
    };

    // Выбор и перенос стрелки за середину.
    s.on_key(Some(KeyCode::KeyV), None, None);
    click(&mut s, 300.0, 250.0);
    c.ok("edit: click picks arrow", s.picked() == Some(0));
    drag(&mut s, 0, (300.0, 250.0), (350.0, 270.0));
    let arrow = |s: &Session| match s.shapes()[0].kind {
        Kind::Arrow(a, b) => Some((a, b)),
        _ => None,
    };
    c.ok("edit: arrow moved", arrow(&s) == Some(((250.0, 220.0), (450.0, 320.0))));
    ctrl_z(&mut s, false);
    c.ok("edit: undo move", arrow(&s) == Some(((200.0, 200.0), (400.0, 300.0))));
    ctrl_z(&mut s, true);
    c.ok("edit: redo move", arrow(&s) == Some(((250.0, 220.0), (450.0, 320.0))));

    // Ручка: конец стрелки.
    click(&mut s, 350.0, 270.0);
    drag(&mut s, 0, (450.0, 320.0), (500.0, 400.0));
    c.ok("edit: handle moves arrow tip", arrow(&s).map(|x| x.1) == Some((500.0, 400.0)));
    c.ok("edit: handle keeps arrow tail", arrow(&s).map(|x| x.0) == Some((250.0, 220.0)));

    // Рамка: выбор по контуру, клик внутри мимо контура не выбирает.
    click(&mut s, 525.0, 220.0);
    c.ok("edit: click inside empty rect misses", s.picked().is_none());
    click(&mut s, 450.0, 220.0);
    c.ok("edit: click on rect edge picks it", s.picked() == Some(1));
    // Угол рамки.
    drag(&mut s, 0, (600.0, 260.0), (620.0, 300.0));
    c.ok("edit: rect corner resized", matches!(s.shapes()[1].kind, Kind::Rect(a, b) if a == (450.0, 180.0) && b == (620.0, 300.0)));
    // Колесо: толщина выбранной, серия отменяется разом.
    let w0 = s.shapes()[1].width;
    s.on_wheel(1.0);
    s.on_wheel(1.0);
    c.ok("edit: wheel widens picked", s.shapes()[1].width == w0 + 2.0);
    c.ok("edit: wheel on picked keeps brush width", s.width == 4.0);
    ctrl_z(&mut s, false);
    c.ok("edit: wheel series undone at once", s.shapes()[1].width == w0);
    // Delete и отмена.
    click(&mut s, 450.0, 220.0);
    s.on_key(None, Some(NamedKey::Delete), None);
    c.ok("edit: delete removes", s.shapes().len() == 3);
    ctrl_z(&mut s, false);
    c.ok("edit: undo delete", s.shapes().len() == 4 && matches!(s.shapes()[1].kind, Kind::Rect(..)));

    // Пикселизация: выбор по площади, стрелки двигают, Shift на 10.
    click(&mut s, 250.0, 380.0);
    c.ok("edit: click picks pixelate", s.picked() == Some(2));
    s.on_key(None, Some(NamedKey::ArrowRight), None);
    s.mods = Mods { shift: true, ..Default::default() };
    s.on_key(None, Some(NamedKey::ArrowDown), None);
    s.mods = Mods::default();
    c.ok("edit: arrows nudge pixelate", matches!(s.shapes()[2].kind, Kind::Pixelate(a, _) if a == (201.0, 340.0)));
    ctrl_z(&mut s, false);
    c.ok("edit: nudge series undone at once", matches!(s.shapes()[2].kind, Kind::Pixelate(a, _) if a == (200.0, 330.0)));
    let frame = s.render(0).clone();
    click(&mut s, 250.0, 380.0);
    let frame_picked = s.render(0).clone();
    output::save_png(&frame_picked, &dir.join("frame_shape_picked.png")).ok();
    c.ok("edit: picked shape outlined", frame.data() != frame_picked.data());
    // Esc снимает выбор, а не закрывает.
    c.ok("edit: esc deselects", s.on_key(None, Some(NamedKey::Escape), None) == Action::None && s.picked().is_none());

    // Двойной клик по тексту: правка на месте, стиль и место в списке те же.
    let (tx, ty) = (text_at.0 + 5.0, text_at.1 + 8.0);
    click(&mut s, tx, ty);
    click(&mut s, tx, ty);
    s.on_key(Some(KeyCode::Digit1), None, Some("!"));
    s.on_key(None, Some(NamedKey::Escape), None);
    c.ok("edit: text re-edited in place", matches!(&s.shapes()[3].kind, Kind::Text { text, at } if text == "Hi!" && *at == text_at));
    ctrl_z(&mut s, false);
    c.ok("edit: undo text edit", matches!(&s.shapes()[3].kind, Kind::Text { text, .. } if text == "Hi"));

    // Курсор не трогает выделение: протяжка по пустому месту ничего не выделяет.
    let before = s.result().unwrap();
    drag(&mut s, 0, (100.0, 100.0), (800.0, 600.0));
    let after = s.result().unwrap();
    c.ok("edit: pointer drag keeps selection", (after.width(), after.height()) == (before.width(), before.height()));

    // Новая область на том же мониторе (рамка, M): разметка остаётся.
    s.on_key(Some(KeyCode::KeyM), None, None);
    drag(&mut s, 0, (100.0, 100.0), (800.0, 600.0));
    c.ok("edit: new area keeps shapes", s.shapes().len() == 4);
    let img = s.result().unwrap();
    c.ok("edit: new area size", (img.width(), img.height()) == (700, 500));

    // Панель инструментов тянут за заголовок, двойной клик по нему возвращает её.
    let r0 = s.tools_rect().unwrap();
    let g = (r0.left() + r0.width() / 2.0, r0.top() + 4.0);
    drag(&mut s, 0, g, (g.0 - 300.0, g.1 + 40.0));
    let r1 = s.tools_rect().unwrap();
    c.ok("panel dragged by its title", (r1.left() - (r0.left() - 300.0)).abs() < 1.0 && (r1.top() - (r0.top() + 40.0)).abs() < 1.0);
    c.ok("panel drag keeps shapes and selection", s.shapes().len() == 4 && s.result().is_some_and(|i| (i.width(), i.height()) == (700, 500)));
    let g1 = (r1.left() + r1.width() / 2.0, r1.top() + 4.0);
    click(&mut s, g1.0, g1.1);
    click(&mut s, g1.0, g1.1);
    c.ok("panel double click returns it", s.tools_rect() == Some(r0));

    // Проект: после открытия Ctrl+Z снимает фигуры по одной.
    let bytes = s.to_project().unwrap();
    let mut p = Session::from_project(&bytes, 0, 0, 1.0, 0.5, None).unwrap();
    ctrl_z(&mut p, false);
    c.ok("edit: project undo removes last shape", p.shapes().len() == 3);
}

/// Исправления по код-ревью v0.1.4: каждое находкой воспроизводится и проверяется.
fn fixes_check(c: &mut Check, font: Option<Arc<ab_glyph::FontVec>>) {
    use crate::shapes::{Kind, Shape};
    let fresh = |font: Option<Arc<ab_glyph::FontVec>>| {
        let mut img = tiny_skia::Pixmap::new(1000, 800).unwrap();
        img.fill(tiny_skia::Color::from_rgba8(60, 90, 140, 255));
        let shot = crate::capture::MonitorShot { x: 0, y: 0, pixmap: img };
        let mut s = Session::new(vec![shot], vec![1.0], 0.5, 0xE24B4A, 4.0, font);
        drag(&mut s, 0, (150.0, 150.0), (650.0, 450.0));
        s
    };
    let click = |s: &mut Session, x: f32, y: f32| {
        s.on_left_press(0, x, y);
        s.on_left_release(0, x, y);
    };
    let ctrl_z = |s: &mut Session, shift: bool| {
        s.mods = Mods { ctrl: true, shift, ..Default::default() };
        s.on_key(Some(KeyCode::KeyZ), None, None);
        s.mods = Mods::default();
    };
    let rect_of = |s: &Session, i: usize| match s.shapes()[i].kind {
        Kind::Rect(a, b) => Some((a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))),
        _ => None,
    };

    // 1. Огромный проект из недоверенного файла открывается быстро, Ctrl+Z снимает по одной.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 200.0), (300.0, 200.0));
        let (mut h, shot) = s.project_parts().unwrap();
        let pencil = Shape { kind: Kind::Pencil((0..190_000).map(|i| ((i % 900) as f32, (i / 900) as f32)).collect()), color: [1, 2, 3], width: 2.0 };
        h.shapes = std::iter::once(pencil)
            .chain((0..9_999).map(|i| Shape { kind: Kind::Counter { at: (200.0, 200.0), n: i, tip: None }, color: [1, 2, 3], width: 4.0 }))
            .collect();
        let bytes = crate::project::build(h, &shot).unwrap();
        let t0 = std::time::Instant::now();
        let mut p = Session::from_project(&bytes, 0, 0, 1.0, 0.5, None).unwrap();
        let took = t0.elapsed();
        println!("INFO 10000-shape project opened in {took:?}");
        c.ok("fix: huge project opens fast", took < std::time::Duration::from_secs(3));
        ctrl_z(&mut p, false);
        ctrl_z(&mut p, false);
        c.ok("fix: project undo pops loaded shapes one by one", p.shapes().len() == 9_998);
        ctrl_z(&mut p, true);
        c.ok("fix: project redo restores popped shape", p.shapes().len() == 9_999);
    }

    // 5. Угол рамки за противоположную сторону: рамка разворачивается, а не схлопывается.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit5), None, None);
        drag(&mut s, 0, (200.0, 200.0), (300.0, 300.0));
        s.on_key(Some(KeyCode::KeyV), None, None);
        click(&mut s, 200.0, 250.0);
        s.on_left_press(0, 200.0, 200.0);
        for x in [250.0, 330.0, 360.0, 400.0] {
            s.on_move(0, x, 260.0);
        }
        s.on_left_release(0, 400.0, 260.0);
        c.ok("fix: corner dragged past the opposite edge", rect_of(&s, 0) == Some((300.0, 260.0, 400.0, 300.0)));
    }

    // 6. Новая область, отменённая Esc, не теряет разметку и её отмену.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit5), None, None);
        drag(&mut s, 0, (200.0, 200.0), (300.0, 300.0));
        s.on_key(Some(KeyCode::KeyM), None, None);
        s.on_left_press(0, 50.0, 50.0);
        s.on_move(0, 120.0, 120.0);
        s.on_key(None, Some(NamedKey::Escape), None);
        drag(&mut s, 0, (100.0, 100.0), (700.0, 500.0));
        c.ok("fix: cancelled area keeps shapes", s.shapes().len() == 1);
        ctrl_z(&mut s, false);
        c.ok("fix: cancelled area keeps undo", s.shapes().is_empty());
    }

    // 7 и 10. Правка текста: без изменений нет шага отмены и повтор цел; автоскрытие во время правки.
    {
        let mut s = fresh(font.clone());
        s.on_key(Some(KeyCode::Digit6), None, None);
        click(&mut s, 300.0, 300.0);
        s.on_key(Some(KeyCode::KeyH), None, Some("Hi"));
        s.on_key(None, Some(NamedKey::Escape), None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 400.0), (400.0, 400.0));
        ctrl_z(&mut s, false);
        let text_at = match s.shapes()[0].kind {
            Kind::Text { at, .. } => at,
            _ => (0.0, 0.0),
        };
        s.on_key(Some(KeyCode::KeyV), None, None);
        let (tx, ty) = (text_at.0 + 4.0, text_at.1 + 8.0);
        click(&mut s, tx, ty);
        click(&mut s, tx, ty);
        s.on_key(None, Some(NamedKey::Escape), None);
        ctrl_z(&mut s, true);
        c.ok("fix: opening text without change keeps redo", s.shapes().len() == 2);
        // Автоскрытие приходит, пока текст открыт на правку.
        click(&mut s, tx, ty);
        click(&mut s, tx, ty);
        let hidden = s.apply_hide(&[tiny_skia::Rect::from_xywh(500.0, 200.0, 50.0, 20.0).unwrap()]);
        // Редактор остаётся открытым; закрыть без изменений, затем отмена скрытия.
        s.on_key(None, Some(NamedKey::Escape), None);
        ctrl_z(&mut s, false);
        let has_text = s.shapes().iter().any(|x| matches!(x.kind, Kind::Text { .. }));
        let pix = s.shapes().iter().filter(|x| matches!(x.kind, Kind::Pixelate(..))).count();
        c.ok("fix: auto-hide during text edit, undo keeps text", hidden == 1 && has_text && pix == 0);
        ctrl_z(&mut s, true);
        c.ok("fix: auto-hide during text edit, redo returns it", s.shapes().iter().any(|x| matches!(x.kind, Kind::Pixelate(..))));
    }

    // 9. Во время переноса фигуры Ctrl+Z и колесо не действуют, Esc возвращает её на место.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 200.0), (300.0, 200.0));
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 300.0), (300.0, 300.0));
        s.on_key(Some(KeyCode::KeyV), None, None);
        s.on_left_press(0, 250.0, 300.0);
        s.on_move(0, 260.0, 340.0);
        ctrl_z(&mut s, false);
        s.on_wheel(1.0);
        s.on_key(None, Some(NamedKey::Escape), None);
        let back = matches!(s.shapes()[1].kind, Kind::Line(a, _) if a == (200.0, 300.0)) && s.shapes()[1].width == 4.0;
        c.ok("fix: cancelled drag restores the shape", back && s.shapes().len() == 2);
        s.on_left_release(0, 260.0, 340.0);
        ctrl_z(&mut s, false);
        c.ok("fix: undo after cancelled drag undoes the previous step", s.shapes().len() == 1);
    }

    // 14. Подпись линейки ловит клик, наконечник стрелки в рамке выбора.
    if let Some(f) = font.clone() {
        let ruler = Shape { kind: Kind::Ruler((100.0, 100.0), (300.0, 100.0)), color: [255, 0, 0], width: 4.0 };
        let pl = crate::shapes::ruler_plate((100.0, 100.0), (300.0, 100.0), 4.0, &f, Some(0.25)).unwrap();
        let center = (pl.x + pl.w / 2.0, pl.y + pl.h / 2.0);
        c.ok("fix: ruler label is clickable", ruler.hit(center, 1.0, Some(&f), Some(0.25)));
        let (_, t, _, b) = ruler.bounds(Some(&f), Some(0.25));
        c.ok("fix: ruler bounds cover ticks and label", t <= 100.0 - 10.0 && b >= pl.y + pl.h);
    }
    let arrow = Shape { kind: Kind::Arrow((100.0, 100.0), (300.0, 100.0)), color: [255, 0, 0], width: 10.0 };
    let (_, t, _, b) = arrow.bounds(None, None);
    c.ok("fix: arrow bounds cover the head", t <= 80.0 && b >= 120.0);

    // 15. Вытянутый эллипс не ловит клики за своими концами.
    let el = Shape { kind: Kind::Ellipse((100.0, 100.0), (500.0, 140.0)), color: [255, 0, 0], width: 4.0 };
    c.ok("fix: ellipse tip hit", el.hit((500.0, 120.0), 5.0, None, None));
    c.ok("fix: no ellipse hit far past the tip", !el.hit((560.0, 120.0), 5.0, None, None));

    // 16. Радиус захвата линии задаётся в экранных пикселях (tol), без 6 px снимка.
    let line = Shape { kind: Kind::Line((100.0, 100.0), (300.0, 100.0)), color: [255, 0, 0], width: 2.0 };
    c.ok("fix: zoomed line grab radius", !line.hit((200.0, 104.0), 0.3, None, None) && line.hit((200.0, 101.0), 0.3, None, None));

    // 18. Стрелки клавиатуры не уводят фигуру с монитора.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (600.0, 200.0), (640.0, 200.0));
        s.on_key(Some(KeyCode::KeyV), None, None);
        click(&mut s, 620.0, 200.0);
        s.mods = Mods { shift: true, ..Default::default() };
        for _ in 0..500 {
            s.on_key(None, Some(NamedKey::ArrowRight), None);
        }
        s.mods = Mods::default();
        let (l, ..) = s.shapes()[0].bounds(None, None);
        c.ok("fix: nudge stays on the monitor", l <= 1000.0);
    }

    // 4. Клик достаётся верхней панели: панель инструментов перетащена на панель
    //    действий, открыты окно стиля и меню сохранения. Центр любой кнопки попадает в
    //    неё саму или в кнопку панели выше; кнопки окна стиля и меню всегда в себя.
    let bbox = tiny_skia::Rect::from_xywh(600.0, 300.0, 600.0, 324.0).unwrap();
    let (mut overlapping, mut consistent, mut top_win) = (0, true, true);
    for gx in 0..24 {
        for gy in 0..14 {
            let l = crate::ui::layout(&crate::ui::LayoutArgs {
                bbox,
                mw: 1920.0,
                mh: 1080.0,
                s: 1.0,
                save_menu: true,
                style_pop: Some(Some(crate::shapes::Tool::Arrow)),
                tools_at: Some((gx as f32 * 80.0, gy as f32 * 80.0)),
                cols: 2,
                font: None,
            });
            let pairs: Vec<_> = l.buttons.iter().zip(&l.owner).collect();
            if pairs.iter().any(|((_, r), o)| pairs.iter().any(|((_, q), o2)| o2 > o && q.intersect(r).is_some())) {
                overlapping += 1;
            }
            // Кнопка нажимается сама, если её не закрывает панель выше; закрыта: клик
            // достаётся верхней панели (её кнопке или пустому месту на ней).
            consistent &= pairs.iter().all(|((b, r), o)| {
                let (x, y) = (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);
                let covered = l.panels.iter().enumerate().any(|(i, p)| i > **o && x >= p.left() && x < p.right() && y >= p.top() && y < p.bottom());
                let hit = l.hit(x, y);
                if covered { hit != Some(*b) && hit.is_none_or(|h| pairs.iter().any(|((b2, _), o2)| *b2 == h && o2 > o)) } else { hit == Some(*b) }
            });
            top_win &= l.buttons.iter().zip(&l.owner).filter(|(_, o)| **o == l.panels.len() - 1).all(|((b, r), _)| l.hit(r.left() + r.width() / 2.0, r.top() + r.height() / 2.0) == Some(*b));
        }
    }
    println!("INFO layouts with overlapping panels: {overlapping} of 336");
    let consistent = consistent && overlapping > 0;
    c.ok("fix: clicks go to the topmost panel", consistent && top_win);

    // 11. Сразу после переноса панели её снова можно тащить (не двойной клик).
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        let r0 = s.tools_rect().unwrap();
        let g = (r0.left() + r0.width() / 2.0, r0.top() + 4.0);
        drag(&mut s, 0, g, (g.0 - 200.0, g.1));
        let r1 = s.tools_rect().unwrap();
        let g1 = (r1.left() + r1.width() / 2.0, r1.top() + 4.0);
        drag(&mut s, 0, g1, (g1.0 - 100.0, g1.1));
        let r2 = s.tools_rect().unwrap();
        c.ok("fix: second quick grip drag moves the panel", (r2.left() - (r1.left() - 100.0)).abs() < 1.0);
    }

    // 8. Проект без масштаба линейки: только пиксели, не миллиметры монитора, где открыли.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::KeyR), None, None);
        drag(&mut s, 0, (200.0, 200.0), (400.0, 200.0));
        let (mut h, shot) = s.project_parts().unwrap();
        h.mm_per_px = None;
        let bytes = crate::project::build(h, &shot).unwrap();
        let mut p = Session::from_project(&bytes, 0, 0, 1.0, 0.5, None).unwrap();
        let svg = p.to_svg(false).unwrap();
        c.ok("fix: old project ruler stays in pixels", svg.contains("200 px") && !svg.contains("мм"));
    }
}

/// Второй раунд: находки проверки исправлений.
fn fixes2_check(c: &mut Check, font: Option<Arc<ab_glyph::FontVec>>) {
    use crate::shapes::{Kind, Shape};
    let fresh = |font: Option<Arc<ab_glyph::FontVec>>| {
        let mut img = tiny_skia::Pixmap::new(1000, 800).unwrap();
        img.fill(tiny_skia::Color::from_rgba8(60, 90, 140, 255));
        let shot = crate::capture::MonitorShot { x: 0, y: 0, pixmap: img };
        let mut s = Session::new(vec![shot], vec![1.0], 0.5, 0xE24B4A, 4.0, font);
        drag(&mut s, 0, (150.0, 150.0), (650.0, 450.0));
        s
    };
    let click = |s: &mut Session, x: f32, y: f32| {
        s.on_left_press(0, x, y);
        s.on_left_release(0, x, y);
    };
    let ctrl_z = |s: &mut Session, shift: bool| {
        s.mods = Mods { ctrl: true, shift, ..Default::default() };
        s.on_key(Some(KeyCode::KeyZ), None, None);
        s.mods = Mods::default();
    };
    let count = |s: &Session, f: fn(&Kind) -> bool| s.shapes().iter().filter(|x| f(&x.kind)).count();

    // Снятие фигур большого проекта по одной не копирует весь список.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 200.0), (300.0, 200.0));
        let (mut h, shot) = s.project_parts().unwrap();
        let pencil = Shape { kind: Kind::Pencil((0..190_000).map(|i| ((i % 900) as f32, (i / 900) as f32)).collect()), color: [1, 2, 3], width: 2.0 };
        h.shapes = std::iter::once(pencil)
            .chain((0..9_999).map(|i| Shape { kind: Kind::Counter { at: (200.0, 200.0), n: i, tip: None }, color: [1, 2, 3], width: 4.0 }))
            .collect();
        let bytes = crate::project::build(h, &shot).unwrap();
        let mut p = Session::from_project(&bytes, 0, 0, 1.0, 0.5, None).unwrap();
        let t0 = std::time::Instant::now();
        for _ in 0..3000 {
            ctrl_z(&mut p, false);
        }
        for _ in 0..1000 {
            ctrl_z(&mut p, true);
        }
        let took = t0.elapsed();
        println!("INFO 3000 undo + 1000 redo on a 10000-shape project: {took:?}");
        c.ok("fix2: project undo/redo without list copies", took < std::time::Duration::from_secs(2) && p.shapes().len() == 8_000);
    }

    // Автоскрытие во время правки текста не закрывает редактор; отмена по шагам.
    {
        let mut s = fresh(font.clone());
        s.on_key(Some(KeyCode::Digit6), None, None);
        click(&mut s, 300.0, 300.0);
        s.on_key(Some(KeyCode::KeyH), None, Some("Hi"));
        s.on_key(None, Some(NamedKey::Escape), None);
        let at = match s.shapes()[0].kind {
            Kind::Text { at, .. } => at,
            _ => (0.0, 0.0),
        };
        s.on_key(Some(KeyCode::KeyV), None, None);
        click(&mut s, at.0 + 4.0, at.1 + 8.0);
        click(&mut s, at.0 + 4.0, at.1 + 8.0);
        s.apply_hide(&[tiny_skia::Rect::from_xywh(500.0, 200.0, 50.0, 20.0).unwrap()]);
        // Буква p во время правки: символ текста, а не «закрепить».
        let act = s.on_key(Some(KeyCode::KeyP), None, Some("p"));
        s.on_key(None, Some(NamedKey::Escape), None);
        let text_is = |s: &Session, want: &str| s.shapes().iter().any(|x| matches!(&x.kind, Kind::Text { text, .. } if text == want));
        c.ok("fix2: auto-hide keeps the text editor open", act == Action::None && text_is(&s, "Hip"));
        ctrl_z(&mut s, false);
        c.ok("fix2: undo text edit keeps auto-hide", text_is(&s, "Hi") && count(&s, |k| matches!(k, Kind::Pixelate(..))) == 1);
        ctrl_z(&mut s, false);
        c.ok("fix2: next undo removes auto-hide, text stays", text_is(&s, "Hi") && count(&s, |k| matches!(k, Kind::Pixelate(..))) == 0);
    }

    // Автоскрытие во время переноса фигуры применяется после него, отдельным шагом.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 300.0), (300.0, 300.0));
        s.on_key(Some(KeyCode::KeyV), None, None);
        s.on_left_press(0, 250.0, 300.0);
        s.on_move(0, 260.0, 340.0);
        let n = s.apply_hide(&[tiny_skia::Rect::from_xywh(500.0, 200.0, 50.0, 20.0).unwrap()]);
        let during = count(&s, |k| matches!(k, Kind::Pixelate(..)));
        s.on_left_release(0, 260.0, 340.0);
        let after = count(&s, |k| matches!(k, Kind::Pixelate(..)));
        c.ok("fix2: auto-hide waits for the drag", n == 1 && during == 0 && after == 1);
        ctrl_z(&mut s, false);
        let moved = matches!(s.shapes()[0].kind, Kind::Line(a, _) if a == (210.0, 340.0));
        c.ok("fix2: undo removes auto-hide, keeps the move", moved && count(&s, |k| matches!(k, Kind::Pixelate(..))) == 0);
        ctrl_z(&mut s, false);
        c.ok("fix2: next undo reverts the move", matches!(s.shapes()[0].kind, Kind::Line(a, _) if a == (200.0, 300.0)));
    }

    // Esc при переносе фигуры не стирает ветку повтора.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 300.0), (300.0, 300.0));
        drag(&mut s, 0, (200.0, 400.0), (300.0, 400.0));
        ctrl_z(&mut s, false);
        s.on_key(Some(KeyCode::KeyV), None, None);
        s.on_left_press(0, 250.0, 300.0);
        s.on_move(0, 251.0, 300.0);
        s.on_key(None, Some(NamedKey::Escape), None);
        s.on_left_release(0, 251.0, 300.0);
        ctrl_z(&mut s, true);
        c.ok("fix2: cancelled drag keeps redo", s.shapes().len() == 2);
    }

    // Двойной клик по заголовку срабатывает, даже если мышь дрогнула на 1-2 px.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        let r0 = s.tools_rect().unwrap();
        let g = (r0.left() + r0.width() / 2.0, r0.top() + 4.0);
        drag(&mut s, 0, g, (g.0 - 200.0, g.1));
        let r1 = s.tools_rect().unwrap();
        let g1 = (r1.left() + r1.width() / 2.0, r1.top() + 4.0);
        s.on_left_press(0, g1.0, g1.1);
        s.on_move(0, g1.0 + 1.0, g1.1 + 1.0);
        s.on_left_release(0, g1.0 + 1.0, g1.1 + 1.0);
        click(&mut s, g1.0 + 1.0, g1.1 + 1.0);
        c.ok("fix2: grip double click survives 1 px jitter", s.tools_rect() == Some(r0));
    }

    // Без выделения Ctrl+Z ничего не меняет вслепую.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 300.0), (300.0, 300.0));
        s.on_key(Some(KeyCode::KeyM), None, None);
        s.on_left_press(0, 50.0, 50.0);
        s.on_move(0, 120.0, 120.0);
        s.on_key(None, Some(NamedKey::Escape), None);
        ctrl_z(&mut s, false);
        drag(&mut s, 0, (100.0, 100.0), (700.0, 500.0));
        c.ok("fix2: no blind undo without an area", s.shapes().len() == 1);
    }

    // Геометрия: тонкий и маленький эллипс, наконечник стрелки, засечки линейки.
    let el = |a: (f32, f32), b: (f32, f32)| Shape { kind: Kind::Ellipse(a, b), color: [255, 0, 0], width: 4.0 };
    c.ok("fix2: flat ellipse hit on its line", el((100.0, 100.0), (300.0, 100.0)).hit((150.0, 100.0), 5.0, None, None));
    c.ok("fix2: small circle hit radius", !el((100.0, 100.0), (103.0, 103.0)).hit((111.5, 101.5), 5.0, None, None));
    c.ok("fix2: small circle hit on its outline", el((100.0, 100.0), (110.0, 110.0)).hit((110.0, 105.0), 5.0, None, None));
    let arrow = Shape { kind: Kind::Arrow((100.0, 100.0), (300.0, 100.0)), color: [255, 0, 0], width: 10.0 };
    c.ok("fix2: arrow head triangle hit", arrow.hit((265.0, 85.0), 5.0, None, None) && !arrow.hit((320.0, 100.0), 5.0, None, None));
    let thin = Shape { kind: Kind::Arrow((100.0, 100.0), (300.0, 100.0)), color: [255, 0, 0], width: 2.0 };
    c.ok("fix2: thin arrow head hit when zoomed", thin.hit((288.0, 104.0), 0.3, None, None));
    let ruler = Shape { kind: Kind::Ruler((100.0, 100.0), (300.0, 100.0)), color: [255, 0, 0], width: 40.0 };
    let (_, t, _, _) = ruler.bounds(None, None);
    c.ok("fix2: ruler bounds cover round tick caps", t <= 100.0 - 56.0);
    c.ok("fix2: ruler tick end is clickable", ruler.hit((100.0, 45.0), 1.0, None, None));

    // Фигура далеко за краем не губит проект: пропадает только она.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 300.0), (300.0, 300.0));
        let (mut h, shot) = s.project_parts().unwrap();
        h.shapes.push(Shape { kind: Kind::Line((-1.0e6, 10.0), (5.0, 5.0)), color: [1, 2, 3], width: 4.0 });
        let bytes = crate::project::build(h, &shot).unwrap();
        let p = Session::from_project(&bytes, 0, 0, 1.0, 0.5, None);
        c.ok("fix2: far shape dropped, project opens", p.is_ok_and(|p| p.shapes().len() == 1));
    }

    // Третий круг. Esc при переносе с полной историей не съедает старую запись.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        for i in 0..205 {
            let y = 160.0 + i as f32;
            drag(&mut s, 0, (200.0, y), (240.0, y));
        }
        s.on_key(Some(KeyCode::KeyV), None, None);
        s.on_left_press(0, 220.0, 200.0);
        s.on_move(0, 221.0, 200.0);
        s.on_key(None, Some(NamedKey::Escape), None);
        s.on_left_release(0, 221.0, 200.0);
        for _ in 0..250 {
            ctrl_z(&mut s, false);
        }
        c.ok("fix3: cancelled drag at the undo cap keeps 200 records", s.shapes().len() == 5);
    }
    // Усыпление посреди переноса: отложенное автоскрытие применяется.
    {
        let mut s = fresh(None);
        s.on_key(Some(KeyCode::Digit3), None, None);
        drag(&mut s, 0, (200.0, 300.0), (300.0, 300.0));
        s.on_key(Some(KeyCode::KeyV), None, None);
        s.on_left_press(0, 250.0, 300.0);
        s.on_move(0, 260.0, 340.0);
        s.apply_hide(&[tiny_skia::Rect::from_xywh(500.0, 200.0, 50.0, 20.0).unwrap()]);
        s.hibernate();
        s.wake();
        c.ok("fix3: hibernate mid-drag applies pending auto-hide", count(&s, |k| matches!(k, Kind::Pixelate(..))) == 1);
    }
    let fat = Shape { kind: Kind::Arrow((100.0, 300.0), (500.0, 300.0)), color: [255, 0, 0], width: 40.0 };
    c.ok("fix3: no arrow hit past its tip", !fat.hit((520.0, 300.0), 5.0, None, None) && fat.hit((480.0, 300.0), 5.0, None, None));
    let big = Shape { kind: Kind::Ellipse((0.0, 100.0), (3840.0, 2060.0)), color: [255, 0, 0], width: 1.0 };
    let th = 2.5f32.to_radians();
    c.ok("fix3: big ellipse hit on its curve when zoomed", big.hit((1920.0 + 1920.0 * th.cos(), 1080.0 + 980.0 * th.sin()), 0.9375, None, None));

    // Стрелки клавиатуры: целые шаги, фигура остаётся касаться монитора.
    {
        let mut s = fresh(None);
        s.width = 3.0;
        s.on_key(Some(KeyCode::Digit5), None, None);
        drag(&mut s, 0, (600.0, 200.0), (640.0, 240.0));
        s.on_key(Some(KeyCode::KeyV), None, None);
        click(&mut s, 600.0, 220.0);
        s.mods = Mods { shift: true, ..Default::default() };
        for _ in 0..100 {
            s.on_key(None, Some(NamedKey::ArrowRight), None);
        }
        s.mods = Mods::default();
        // Край штриха (a.x - 1,5) упирается в край монитора, шаги целые.
        let ok = matches!(s.shapes()[0].kind, Kind::Rect(a, _) if a.0 == a.0.round() && a.0 - 1.5 <= 1000.0 && a.0 > 990.0);
        c.ok("fix2: nudge keeps whole pixels and stays on the monitor", ok);
    }
}

/// Панели в стиле Photoshop: окно стиля, готовые размеры, колонки, подсказка.
fn panels_check(c: &mut Check, dir: &Path, font: Option<Arc<ab_glyph::FontVec>>) {
    use crate::ui::Btn;
    let mut img = tiny_skia::Pixmap::new(1600, 900).unwrap();
    img.fill(tiny_skia::Color::from_rgba8(60, 90, 140, 255));
    let shot = crate::capture::MonitorShot { x: 0, y: 0, pixmap: img };
    let mut s = Session::new(vec![shot], vec![1.0], 0.5, 0xE24B4A, 4.0, font);
    drag(&mut s, 0, (200.0, 150.0), (900.0, 600.0));
    s.on_key(Some(KeyCode::Digit4), None, None);
    let center = |r: tiny_skia::Rect| (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);
    let click = |s: &mut Session, b: Btn| {
        let Some(r) = s.button_rect(b) else { return Action::None };
        let (x, y) = center(r);
        let a = s.on_left_press(0, x, y);
        s.on_left_release(0, x, y);
        a
    };
    click(&mut s, Btn::Style);
    c.ok("panels: style popover opens", s.button_rect(Btn::Swatch(0)).is_some() && s.button_rect(Btn::Preset(4)).is_some());
    click(&mut s, Btn::Swatch(3));
    c.ok("panels: swatch sets color, popover stays", s.color == crate::ui::PALETTE[3] && s.button_rect(Btn::Preset(0)).is_some());
    click(&mut s, Btn::Preset(4));
    c.ok("panels: preset sets width", s.width == 16.0);
    // Смена инструмента не закрывает окно, размеры под новый инструмент; у
    // закрашенного прямоугольника размеров нет, только цвета.
    s.on_key(Some(KeyCode::Digit6), None, None);
    let text_presets = s.button_rect(Btn::Preset(0)).is_some();
    s.on_key(Some(KeyCode::Digit8), None, None);
    let frect_colors_only = s.button_rect(Btn::Swatch(0)).is_some() && s.button_rect(Btn::Preset(0)).is_none();
    s.on_key(Some(KeyCode::Digit4), None, None);
    c.ok("panels: popover follows the tool", text_presets && frect_colors_only && s.button_rect(Btn::Preset(4)).is_some());
    // Подсказка у карандаша и открытое окно стиля: кадр для глаза.
    if let Some(r) = s.button_rect(Btn::Tool(crate::shapes::Tool::Pencil)) {
        let (x, y) = center(r);
        s.on_move(0, x, y);
    }
    let frame = s.render(0).clone();
    output::save_png(&frame, &dir.join("frame_panels.png")).ok();
    // Колесо над выделением: число у курсора (кадр), плитка показывает толщину.
    s.on_move(0, 500.0, 400.0);
    s.on_wheel(1.0);
    c.ok("panels: wheel changes width", s.width == 17.0);
    let frame = s.render(0).clone();
    output::save_png(&frame, &dir.join("frame_wheel_chip.png")).ok();
    // Esc закрывает окно стиля, а не снимок.
    c.ok("panels: esc closes the popover first", s.on_key(None, Some(NamedKey::Escape), None) == Action::None && s.button_rect(Btn::Swatch(0)).is_none());
    // Двойная стрелка: одна колонка, настройка уходит в App.
    let w2 = s.tools_rect().map(|r| r.width());
    let act = click(&mut s, Btn::Columns);
    let w1 = s.tools_rect().map(|r| r.width());
    c.ok("panels: columns toggle", act == Action::ToolColumns(1) && w1 < w2);
    let frame = s.render(0).clone();
    output::save_png(&frame, &dir.join("frame_one_column.png")).ok();
    c.ok("panels: columns toggle back", click(&mut s, Btn::Columns) == Action::ToolColumns(2));
}
