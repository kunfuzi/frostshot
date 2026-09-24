use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Основной хоткей захвата, формат global-hotkey: "PrintScreen", "Ctrl+Shift+F12".
    pub hotkey: String,
    /// Запасной хоткей, регистрируется всегда (PrintScreen может быть занят Snipping Tool).
    pub fallback_hotkey: String,
    /// Папка для быстрого сохранения и стартовая папка диалога.
    pub save_dir: PathBuf,
    /// Последний выбранный цвет 0xRRGGBB.
    pub color: u32,
    /// Последняя толщина линии в пикселях.
    pub width: f32,
    /// Предупреждение про PrintScreen и Snipping Tool уже показано.
    pub printscreen_warned: bool,
}

impl Default for Config {
    fn default() -> Self {
        let pictures = directories::UserDirs::new()
            .and_then(|u| u.picture_dir().map(|p| p.to_path_buf()))
            .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join("Pictures")))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            hotkey: "PrintScreen".into(),
            fallback_hotkey: "Ctrl+Shift+F12".into(),
            save_dir: pictures.join("Frostshot"),
            color: 0xE24B4A,
            width: 4.0,
            printscreen_warned: false,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Frostshot").map(|d| d.config_dir().join("config.toml"))
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                log::warn!("config parse error {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => {
                let cfg = Self::default();
                cfg.save();
                cfg
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match toml::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    log::warn!("config write error {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("config serialize error: {e}"),
        }
    }
}
