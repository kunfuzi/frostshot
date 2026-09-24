//! Выделение как маска: объединение и вычитание прямоугольников и лассо.

use tiny_skia::{FillRule, Mask, Path, PathBuilder, Rect, Transform};

#[derive(Clone, Debug)]
pub enum SelShape {
    Rect(Rect),
    Poly(Vec<(f32, f32)>),
}

impl SelShape {
    fn path(&self) -> Option<Path> {
        match self {
            SelShape::Rect(r) => Some(PathBuilder::from_rect(*r)),
            SelShape::Poly(pts) if pts.len() >= 3 => {
                let mut pb = PathBuilder::new();
                pb.move_to(pts[0].0, pts[0].1);
                for p in &pts[1..] {
                    pb.line_to(p.0, p.1);
                }
                pb.close();
                pb.finish()
            }
            SelShape::Poly(_) => None,
        }
    }

    fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            SelShape::Rect(r) => {
                if let Some(n) = Rect::from_xywh(r.x() + dx, r.y() + dy, r.width(), r.height()) {
                    *r = n;
                }
            }
            SelShape::Poly(pts) => {
                for p in pts {
                    p.0 += dx;
                    p.1 += dy;
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct SelOp {
    pub add: bool,
    pub shape: SelShape,
}

/// Габарит в пикселях монитора.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IBox {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}


pub struct Selection {
    w: u32,
    h: u32,
    pub ops: Vec<SelOp>,
    /// Операция, которую пользователь сейчас тянет мышью.
    pub preview: Option<SelOp>,
    mask: Mask,
    bbox: Option<IBox>,
    edges: Vec<(u32, u32)>,
    dirty: bool,
}

impl Selection {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            ops: Vec::new(),
            preview: None,
            mask: Mask::new(w, h).expect("mask size"),
            bbox: None,
            edges: Vec::new(),
            dirty: false,
        }
    }

    pub fn clear(&mut self) {
        self.ops.clear();
        self.preview = None;
        self.dirty = true;
    }

    pub fn touch(&mut self) {
        self.dirty = true;
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        for op in &mut self.ops {
            op.shape.translate(dx, dy);
        }
        self.dirty = true;
    }

    /// Единственный прямоугольник: для него доступны ручки ресайза.
    pub fn single_rect(&self) -> Option<Rect> {
        match self.ops.as_slice() {
            [SelOp { add: true, shape: SelShape::Rect(r) }] if self.preview.is_none() => Some(*r),
            _ => None,
        }
    }

    pub fn ensure(&mut self) {
        if self.dirty {
            self.rebuild();
            self.dirty = false;
        }
    }

    pub fn mask(&self) -> &Mask {
        &self.mask
    }
    pub fn bbox(&self) -> Option<IBox> {
        self.bbox
    }
    pub fn edges(&self) -> &[(u32, u32)] {
        &self.edges
    }
    pub fn is_empty(&self) -> bool {
        self.bbox.is_none()
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        crate::draw::mask_at(&self.mask, x as i32, y as i32) >= 0.5
    }

    fn rebuild(&mut self) {
        let (w, h) = (self.w, self.h);
        self.mask.data_mut().fill(0);
        let mut acc: Option<(u32, u32, u32, u32)> = None; // x0 y0 x1 y1 объединения add-операций

        for op in self.ops.iter().chain(self.preview.iter()) {
            let Some(path) = op.shape.path() else { continue };
            let b = path.bounds();
            let x0 = b.left().floor().clamp(0.0, w as f32) as u32;
            let y0 = b.top().floor().clamp(0.0, h as f32) as u32;
            let x1 = b.right().ceil().clamp(0.0, w as f32) as u32;
            let y1 = b.bottom().ceil().clamp(0.0, h as f32) as u32;
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let (bw, bh) = (x1 - x0, y1 - y0);
            let Some(mut tmp) = Mask::new(bw, bh) else { continue };
            let aa = matches!(op.shape, SelShape::Poly(_));
            tmp.fill_path(
                &path,
                FillRule::Winding,
                aa,
                Transform::from_translate(-(x0 as f32), -(y0 as f32)),
            );
            let t = tmp.data();
            let m = self.mask.data_mut();
            for yy in 0..bh {
                let row_m = ((y0 + yy) * w + x0) as usize;
                let row_t = (yy * bw) as usize;
                for xx in 0..bw as usize {
                    let tv = t[row_t + xx] as u32;
                    let mv = &mut m[row_m + xx];
                    if op.add {
                        *mv = (*mv).max(tv as u8);
                    } else {
                        *mv = (*mv as u32 * (255 - tv) / 255) as u8;
                    }
                }
            }
            if op.add {
                acc = Some(match acc {
                    None => (x0, y0, x1, y1),
                    Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
                });
            }
        }

        // Точный габарит и контур по фактической маске.
        self.bbox = None;
        self.edges.clear();
        let Some((ax0, ay0, ax1, ay1)) = acc else { return };
        let m = self.mask.data();
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for y in ay0..ay1 {
            for x in ax0..ax1 {
                if m[(y * w + x) as usize] > 0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        self.bbox = Some(IBox { x: x0, y: y0, w: x1 - x0, h: y1 - y0 });
        let inside = |x: i64, y: i64| -> bool {
            x >= 0 && y >= 0 && x < w as i64 && y < h as i64 && m[(y as u32 * w + x as u32) as usize] >= 128
        };
        for y in y0..y1 {
            for x in x0..x1 {
                let (xi, yi) = (x as i64, y as i64);
                if inside(xi, yi)
                    && (!inside(xi - 1, yi) || !inside(xi + 1, yi) || !inside(xi, yi - 1) || !inside(xi, yi + 1))
                {
                    self.edges.push((x, y));
                }
            }
        }
    }
}
