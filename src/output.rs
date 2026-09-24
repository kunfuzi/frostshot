//! Вывод результата: буфер обмена и PNG.

use std::path::{Path, PathBuf};
use tiny_skia::Pixmap;

/// Кладёт картинку в буфер синхронно, без delayed rendering (инвариант 2).
pub fn to_clipboard(cb: &mut arboard::Clipboard, img: &Pixmap) -> Result<(), String> {
    let bytes = crate::tray::rgba_straight(img);
    cb.set_image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: bytes.into(),
    })
    .map_err(|e| e.to_string())
}

pub fn default_file_name() -> String {
    chrono::Local::now().format("Frostshot_%Y-%m-%d_%H-%M-%S.png").to_string()
}

pub fn save_png(img: &Pixmap, path: &Path) -> Result<PathBuf, String> {
    let mut path = path.to_path_buf();
    if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("png")) {
        path.set_extension("png");
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let data = img.encode_png().map_err(|e| e.to_string())?;
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    Ok(path)
}
