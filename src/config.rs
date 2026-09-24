use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_TEMPLATE: &str = "Frostshot_%Y-%m-%d_%H-%M-%S";

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
    /// Размер шрифта подсказок и подписей в px при масштабе 100%.
    pub ui_font_size: f32,
    /// Запуск при входе в систему (зеркало реального состояния).
    pub autostart: bool,
    /// При копировании в буфер также сохранять PNG в папку.
    pub save_on_copy: bool,
    /// Шаблон имени файла (strftime), без расширения.
    pub file_template: String,
    /// Затемнение вне выделения, 0.0..0.9.
    pub dim: f32,
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
            ui_font_size: 18.0,
            autostart: false,
            save_on_copy: false,
            file_template: DEFAULT_TEMPLATE.into(),
            dim: 0.5,
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Frostshot").map(|d| d.config_dir().join("config.toml"))
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Self>(&text) {
                Ok(cfg) => {
                    // Дописать ключи, появившиеся в новой версии.
                    if toml::to_string_pretty(&cfg).is_ok_and(|t| t != text) {
                        cfg.save();
                    }
                    cfg
                }
                Err(e) => {
                    log::warn!("config parse error {}: {e}", path.display());
                    Self::default()
                }
            },
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

/// Имя файла по шаблону или описание ошибки шаблона.
pub fn file_name(template: &str) -> Result<String, String> {
    use chrono::format::{Item, StrftimeItems};
    if template.trim().is_empty() {
        return Err("Шаблон пустой".into());
    }
    if let Some(c) = template.chars().find(|c| r#"\/:*?"<>|"#.contains(*c)) {
        return Err(format!("Недопустимый символ «{c}»"));
    }
    if StrftimeItems::new(template).any(|i| matches!(i, Item::Error)) {
        return Err("Неизвестный код после %".into());
    }
    Ok(format!("{}.png", chrono::Local::now().format(template)))
}
