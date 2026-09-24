use tiny_skia::{IntSize, Pixmap};

/// Снимок одного монитора в физических пикселях.
pub struct MonitorShot {
    pub x: i32,
    pub y: i32,
    pub pixmap: Pixmap,
}

impl MonitorShot {
    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }
    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }
}

/// Снимок всех мониторов. Делается до показа оверлея (инвариант 4).
pub fn capture_all() -> Result<Vec<MonitorShot>, String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("enumerate monitors: {e}"))?;
    let mut shots = Vec::with_capacity(monitors.len());
    for m in monitors {
        let (x, y) = (m.x().unwrap_or(0), m.y().unwrap_or(0));
        let img = match m.capture_image() {
            Ok(img) => img,
            Err(e) => {
                log::warn!("capture monitor at {x},{y} failed: {e}");
                continue;
            }
        };
        let (w, h) = (img.width(), img.height());
        let mut data = img.into_raw();
        // Альфа из BitBlt не определена: делаем снимок непрозрачным до любого блендинга.
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let size = IntSize::from_wh(w, h).ok_or("empty monitor")?;
        let pixmap = Pixmap::from_vec(data, size).ok_or("pixmap from capture")?;
        shots.push(MonitorShot { x, y, pixmap });
    }
    if shots.is_empty() {
        return Err("no monitors captured".into());
    }
    Ok(shots)
}
