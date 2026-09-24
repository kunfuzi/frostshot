//! История снимков: последние проекты `.frost` в папке профиля пользователя.
//! Хранится исходный снимок без пикселизации (иначе разметку не поправить), поэтому
//! история отключается в настройках, чистится кнопкой и сама удаляет старое.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

pub struct Entry {
    pub path: PathBuf,
    pub thumb: PathBuf,
    pub modified: SystemTime,
    pub label: String,
}

pub fn dir() -> Option<PathBuf> {
    let d = directories::ProjectDirs::from("", "", "Frostshot")?.data_local_dir().join("history");
    Some(if cfg!(debug_assertions) { d.join("dev") } else { d })
}

/// Миниатюра для панели истории: вписана в max x max без полей.
fn thumbnail(img: &Pixmap, max: u32) -> Option<Pixmap> {
    let k = (max as f32 / img.width() as f32).min(max as f32 / img.height() as f32).min(1.0);
    let (w, h) = ((img.width() as f32 * k).round().max(1.0) as u32, (img.height() as f32 * k).round().max(1.0) as u32);
    let mut t = Pixmap::new(w, h)?;
    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
    t.draw_pixmap(0, 0, img.as_ref(), &paint, Transform::from_scale(k, k), None);
    Some(t)
}

/// Сохранить проект и миниатюру. Вызывать из фонового потока: кодирование PNG небыстрое.
pub fn save(project: &[u8], result: &Pixmap) -> Result<PathBuf, String> {
    let dir = dir().ok_or("нет папки профиля")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    let path = dir.join(format!("{name}.{}", crate::project::EXT));
    std::fs::write(&path, project).map_err(|e| e.to_string())?;
    if let Some(t) = thumbnail(result, 400) {
        let _ = std::fs::write(path.with_extension("png"), t.encode_png().unwrap_or_default());
    }
    // Размер результата для подписи в панели истории.
    let _ = std::fs::write(path.with_extension("txt"), format!("{}×{}", result.width(), result.height()));
    Ok(path)
}

/// Последние снимки, новые первыми.
pub fn list() -> Vec<Entry> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<Entry> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == crate::project::EXT))
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
            let size = std::fs::read_to_string(path.with_extension("txt")).unwrap_or_default();
            let when = chrono::DateTime::<chrono::Local>::from(modified);
            let today = chrono::Local::now().date_naive() == when.date_naive();
            let time = if today { when.format("сегодня %H:%M").to_string() } else { when.format("%d.%m %H:%M").to_string() };
            let label = if size.is_empty() { time } else { format!("{time}  ·  {size}") };
            Some(Entry { thumb: path.with_extension("png"), path, modified, label })
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("png"));
    let _ = std::fs::remove_file(path.with_extension("txt"));
}

/// Оставить не больше max снимков и не старше days дней.
pub fn prune(max: usize, days: u32) {
    let limit = SystemTime::now() - Duration::from_secs(days as u64 * 86_400);
    for (i, e) in list().into_iter().enumerate() {
        if i >= max || e.modified < limit {
            remove(&e.path);
        }
    }
}

pub fn clear() {
    for e in list() {
        remove(&e.path);
    }
}
