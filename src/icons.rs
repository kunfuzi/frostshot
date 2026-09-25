//! Иконки панелей: данные на сетке 24×24 (как SVG) и их отрисовка.
//! Путь понимает абсолютные команды M, L, H, V, C, Q, Z: иконку можно взять
//! из SVG-файла почти без правок. Набор перерисован по эскизу ChatGPT.

use crate::draw::{self, Rgb};
use tiny_skia::{FillRule, LineCap, LineJoin, PathBuilder, Pixmap, Rect, Stroke, Transform};

/// Как рисовать элемент: контур заданной толщины (в единицах сетки) или заливка.
#[derive(Clone, Copy)]
pub enum Mode {
    Stroke(f32),
    Fill,
}

/// Элемент иконки в координатах сетки 24×24.
pub enum El {
    Path(&'static str, Mode),
    /// x, y, ширина, высота, скругление.
    Rect(f32, f32, f32, f32, f32, Mode),
    Circle(f32, f32, f32, Mode),
    Ellipse(f32, f32, f32, f32, Mode),
}

const S: Mode = Mode::Stroke(1.6);
const F: Mode = Mode::Fill;

pub const POINTER: &[El] = &[
    El::Path("M4 2.5L4 17L7.6 13.7L10 19L12.4 17.9L10.1 12.8L14.8 12.6Z", F),
    El::Path("M15.5 17.5H20.5M18 15V20", S),
    El::Path("M21.9 17.5L20.2 16.1V18.9ZM14.1 17.5L15.8 16.1V18.9ZM18 13.6L16.6 15.3H19.4ZM18 21.4L16.6 19.7H19.4Z", F),
];
pub const MARQUEE: &[El] = &[El::Path(
    "M3.5 8V6.5Q3.5 5 5 5H6.5M9 5H11M13 5H15M17.5 5H19Q20.5 5 20.5 6.5V8M20.5 10.8V13.2M20.5 16V17.5Q20.5 19 19 19H17.5M15 19H13M11 19H9M6.5 19H5Q3.5 19 3.5 17.5V16M3.5 13.2V10.8",
    S,
)];
pub const LASSO: &[El] = &[
    El::Path("M12.5 4.5C17.2 4.5 20.5 6.4 20.5 9C20.5 11.6 17.2 13.5 12.5 13.5C7.8 13.5 4.5 11.6 4.5 9C4.5 6.4 7.8 4.5 12.5 4.5Z", S),
    El::Circle(7.2, 13.9, 1.6, S),
    El::Path("M6.7 15.4C6.1 17 6.5 18.8 7.7 20.2", S),
];
pub const RULER: &[El] = &[
    El::Path("M2.5 17.5L17.5 2.5L21.5 6.5L6.5 21.5Z", S),
    El::Path(
        "M4.64 15.36L5.64 16.36M6.79 13.21L8.49 14.91M8.93 11.07L9.93 12.07M11.07 8.93L12.77 10.63M13.21 6.79L14.21 7.79M15.36 4.64L17.06 6.34",
        Mode::Stroke(1.3),
    ),
];
pub const PENCIL: &[El] = &[
    El::Path("M4 20L5 15.8L15.6 5.2Q17 3.8 18.4 5.2L18.8 5.6Q20.2 7 18.8 8.4L8.2 19Z", S),
    El::Path("M13.9 6.9L17.1 10.1M5 15.8L8.2 19", S),
    El::Path("M4 20L4.55 17.7L6.3 19.45Z", F),
];
pub const MARKER: &[El] = &[
    El::Path("M8.2 11.6L15.4 4.4Q16.8 3 18.2 4.4L19.6 5.8Q21 7.2 19.6 8.6L12.4 15.8Z", S),
    El::Path("M8.2 11.6L12.4 15.8L8.8 17.4L4.4 19.6L6.6 15.2Z", F),
];
pub const LINE: &[El] = &[El::Path("M6.2 17.8L17.8 6.2", S), El::Circle(5.0, 19.0, 1.8, F), El::Circle(19.0, 5.0, 1.8, F)];
pub const ARROW: &[El] = &[El::Path("M4.5 19.5L15.3 8.7", S), El::Path("M20 4L12.5 6.4L17.6 11.5Z", F)];
pub const RECT: &[El] = &[El::Rect(3.5, 6.0, 17.0, 12.0, 1.5, S)];
pub const RECT_FILLED: &[El] = &[El::Rect(3.5, 6.0, 17.0, 12.0, 1.2, F)];
pub const ELLIPSE: &[El] = &[El::Ellipse(12.0, 12.0, 8.5, 5.5, S)];
pub const COUNTER: &[El] = &[
    El::Circle(11.0, 10.5, 7.0, S),
    El::Path("M17.4 13.4L19.8 19.8L13.6 17Z", F),
    El::Path("M9.6 8.2L11.6 6.8V14.2", S),
];
pub const TEXT: &[El] = &[El::Path("M5 4.5H19V8.7H17.6L17 6.9H13.6V17.3L15.6 17.9V19.5H8.4V17.9L10.4 17.3V6.9H7L6.4 8.7H5Z", F)];
const C3: f32 = 16.0 / 3.0;
pub const PIXELATE: &[El] = &[
    El::Rect(4.0, 4.0, C3 + 0.02, C3 + 0.02, 0.0, F),
    El::Rect(4.0 + 2.0 * C3, 4.0, C3 + 0.02, C3 + 0.02, 0.0, F),
    El::Rect(4.0 + C3, 4.0 + C3, C3 + 0.02, C3 + 0.02, 0.0, F),
    El::Rect(4.0, 4.0 + 2.0 * C3, C3 + 0.02, C3 + 0.02, 0.0, F),
    El::Rect(4.0 + 2.0 * C3, 4.0 + 2.0 * C3, C3 + 0.02, C3 + 0.02, 0.0, F),
];
pub const AUTO_HIDE: &[El] = &[
    El::Path("M16.5 12V19.2Q16.5 20.5 15.2 20.5H5.3Q4 20.5 4 19.2V4.8Q4 3.5 5.3 3.5H13", S),
    El::Path("M7 7.5H13M7 10.5H11", S),
    El::Rect(6.5, 13.5, 8.5, 4.0, 1.0, S),
    El::Circle(8.6, 15.5, 0.85, F),
    El::Circle(10.75, 15.5, 0.85, F),
    El::Circle(12.9, 15.5, 0.85, F),
    El::Path("M18.5 2.5Q18.9 5.6 22 6Q18.9 6.4 18.5 9.5Q18.1 6.4 15 6Q18.1 5.6 18.5 2.5Z", F),
];
pub const UNDO: &[El] = &[El::Path("M18.5 19V15Q18.5 9.5 13 9.5H8.5", Mode::Stroke(1.8)), El::Path("M3.8 9.5L9.2 5.8V13.2Z", F)];
pub const REDO: &[El] = &[El::Path("M5.5 19V15Q5.5 9.5 11 9.5H15.5", Mode::Stroke(1.8)), El::Path("M20.2 9.5L14.8 5.8V13.2Z", F)];
pub const OCR: &[El] = &[
    El::Path("M3.5 7.5V5Q3.5 3.5 5 3.5H7.5M16.5 3.5H19Q20.5 3.5 20.5 5V7.5M20.5 16.5V19Q20.5 20.5 19 20.5H16.5M7.5 20.5H5Q3.5 20.5 3.5 19V16.5", S),
    El::Path("M6.5 16.5L9.2 8L11.9 16.5M7.4 13.8H11", S),
    El::Path("M13.8 10H17.5M13.8 12.8H17.5M13.8 15.6H16.5", S),
];
pub const PIN: &[El] = &[
    El::Path("M16.66 3.66L20.34 7.34L18.22 9.46L17.37 8.61L14.54 11.44L16.52 13.42L15.25 14.69L9.31 8.75L10.58 7.48L12.56 9.46L15.39 6.63L14.54 5.78Z", F),
    El::Path("M12.3 11.7L4.5 19.5", S),
];
pub const COPY: &[El] = &[
    El::Path("M9 8.3Q9 7 10.3 7H15.3L19.5 11.2V19.2Q19.5 20.5 18.2 20.5H10.3Q9 20.5 9 19.2Z", S),
    El::Path("M15.3 7V11.2H19.5", S),
    El::Path("M6.5 17H5.8Q4.5 17 4.5 15.7V4.8Q4.5 3.5 5.8 3.5H12.7Q14 3.5 14 4.8V5.5", S),
];
pub const SAVE: &[El] = &[
    El::Path("M4.5 5.8Q4.5 4.5 5.8 4.5H16L19.5 8V18.2Q19.5 19.5 18.2 19.5H5.8Q4.5 19.5 4.5 18.2Z", S),
    El::Path("M8 4.5V8.8H15V4.5", S),
    El::Rect(12.2, 5.7, 1.6, 2.0, 0.3, F),
    El::Rect(7.5, 12.8, 9.0, 5.4, 0.8, F),
];
pub const CLOSE: &[El] = &[El::Path("M6 6L18 18M18 6L6 18", Mode::Stroke(1.8))];
/// Уголок меню у кнопки «Сохранить»: рисуется мелко, поэтому линия толще.
pub const CHEVRON_DOWN: &[El] = &[El::Path("M6.5 9.5L12 15L17.5 9.5", Mode::Stroke(2.8))];
/// Двойная стрелка вправо «»» (одна колонка → две); влево рисуется зеркально.
pub const COLUMNS: &[El] = &[El::Path("M5 6L11 12L5 18M12.5 6L18.5 12L12.5 18", Mode::Stroke(2.2))];

/// Нарисовать иконку в квадрат (x, y, size) цветом c. mirror: отразить по горизонтали.
#[allow(clippy::too_many_arguments)]
pub fn draw(pm: &mut Pixmap, icon: &[El], x: f32, y: f32, size: f32, c: Rgb, alpha: f32, mirror: bool) {
    let k = size / 24.0;
    let tx = |px: f32| if mirror { x + (24.0 - px) * k } else { x + px * k };
    let ty = |py: f32| y + py * k;
    let paint = draw::paint(c, alpha);
    for el in icon {
        let (path, mode) = match el {
            El::Path(d, m) => (parse(d, &tx, &ty), *m),
            El::Rect(rx, ry, w, h, r, m) => {
                let (l, rr) = (tx(*rx).min(tx(rx + w)), tx(*rx).max(tx(rx + w)));
                let rect = Rect::from_ltrb(l, ty(*ry), rr, ty(ry + h));
                let p = rect.and_then(|rect| if *r > 0.0 { draw::rounded_rect(rect, r * k) } else { Some(PathBuilder::from_rect(rect)) });
                (p, *m)
            }
            El::Circle(cx, cy, r, m) => (PathBuilder::from_circle(tx(*cx), ty(*cy), r * k), *m),
            El::Ellipse(cx, cy, rx, ry, m) => {
                let rect = Rect::from_ltrb(tx(*cx) - rx * k, ty(cy - ry), tx(*cx) + rx * k, ty(cy + ry));
                (rect.and_then(PathBuilder::from_oval), *m)
            }
        };
        let Some(path) = path else { continue };
        match mode {
            Mode::Fill => pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None),
            Mode::Stroke(w) => {
                let stroke = Stroke { width: w * k, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
                pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
    }
}

/// Путь из строки SVG с абсолютными командами M L H V C Q Z.
fn parse(d: &str, tx: &dyn Fn(f32) -> f32, ty: &dyn Fn(f32) -> f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let mut nums: Vec<f32> = Vec::new();
    let mut cmd = ' ';
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let flush = |cmd: char, nums: &mut Vec<f32>, pb: &mut PathBuilder, cx: &mut f32, cy: &mut f32| {
        let n = nums.as_slice();
        match cmd {
            'M' | 'L' => {
                for (i, p) in n.chunks_exact(2).enumerate() {
                    (*cx, *cy) = (p[0], p[1]);
                    if cmd == 'M' && i == 0 {
                        pb.move_to(tx(*cx), ty(*cy));
                    } else {
                        pb.line_to(tx(*cx), ty(*cy));
                    }
                }
            }
            'H' => {
                for v in n {
                    *cx = *v;
                    pb.line_to(tx(*cx), ty(*cy));
                }
            }
            'V' => {
                for v in n {
                    *cy = *v;
                    pb.line_to(tx(*cx), ty(*cy));
                }
            }
            'C' => {
                for p in n.chunks_exact(6) {
                    pb.cubic_to(tx(p[0]), ty(p[1]), tx(p[2]), ty(p[3]), tx(p[4]), ty(p[5]));
                    (*cx, *cy) = (p[4], p[5]);
                }
            }
            'Q' => {
                for p in n.chunks_exact(4) {
                    pb.quad_to(tx(p[0]), ty(p[1]), tx(p[2]), ty(p[3]));
                    (*cx, *cy) = (p[2], p[3]);
                }
            }
            'Z' => pb.close(),
            _ => {}
        }
        nums.clear();
    };
    let mut num = String::new();
    let push_num = |num: &mut String, nums: &mut Vec<f32>| {
        if !num.is_empty() {
            match num.parse() {
                Ok(v) => nums.push(v),
                Err(_) => debug_assert!(false, "icon path: bad number {num:?} in {d:?}"),
            }
            num.clear();
        }
    };
    for ch in d.chars() {
        match ch {
            'M' | 'L' | 'H' | 'V' | 'C' | 'Q' | 'Z' => {
                push_num(&mut num, &mut nums);
                flush(cmd, &mut nums, &mut pb, &mut cx, &mut cy);
                cmd = ch;
                if ch == 'Z' {
                    flush('Z', &mut nums, &mut pb, &mut cx, &mut cy);
                    cmd = ' ';
                }
            }
            '0'..='9' => num.push(ch),
            // «1.5.5» в сжатых SVG: вторая точка начинает новое число (1.5 и .5).
            '.' => {
                if num.contains('.') {
                    push_num(&mut num, &mut nums);
                }
                num.push(ch);
            }
            '-' => {
                push_num(&mut num, &mut nums);
                num.push(ch);
            }
            _ => push_num(&mut num, &mut nums),
        }
    }
    push_num(&mut num, &mut nums);
    flush(cmd, &mut nums, &mut pb, &mut cx, &mut cy);
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_draws_something() {
        let all: &[&[El]] = &[
            POINTER, MARQUEE, LASSO, RULER, PENCIL, MARKER, LINE, ARROW, RECT, RECT_FILLED, ELLIPSE, COUNTER, TEXT, PIXELATE, AUTO_HIDE, UNDO, REDO,
            OCR, PIN, COPY, SAVE, CLOSE, CHEVRON_DOWN, COLUMNS,
        ];
        for (i, icon) in all.iter().enumerate() {
            let mut pm = Pixmap::new(24, 24).unwrap();
            draw(&mut pm, icon, 0.0, 0.0, 24.0, [255, 255, 255], 1.0, false);
            let lit = pm.data().chunks_exact(4).filter(|p| p[3] > 0).count();
            assert!(lit > 20, "icon {i} draws {lit} px");
            // Всё внутри поля 24×24: крайние строки и столбцы почти пусты.
            let edge = (0..24).filter(|&j| pm.data()[(j * 24) as usize * 4 + 3] > 200).count();
            assert!(edge < 6, "icon {i} touches the left edge");
        }
    }

    #[test]
    fn path_parser_reads_compact_numbers() {
        // «M1.5.5L-2-3»: 1.5 и .5, затем -2 и -3.
        let p = parse("M1.5.5L-2-3", &|x| x, &|y| y).unwrap();
        let b = p.bounds();
        assert_eq!((b.left(), b.top(), b.right(), b.bottom()), (-2.0, -3.0, 1.5, 0.5));
    }

    #[test]
    fn path_parser_handles_all_commands() {
        let p = parse("M1 2L3 4H5V6C7 8 9 10 11 12Q13 14 15 16Z", &|x| x, &|y| y).unwrap();
        let b = p.bounds();
        assert_eq!((b.left(), b.top(), b.right(), b.bottom()), (1.0, 2.0, 15.0, 16.0));
    }
}
