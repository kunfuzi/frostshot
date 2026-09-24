//! Точка расширения для загрузки в облако. Реализаций пока нет.

#![allow(dead_code)]

pub trait Uploader {
    /// Название сервиса для меню.
    fn name(&self) -> &str;
    /// Загрузить PNG, вернуть публичную ссылку.
    fn upload(&self, png: &[u8]) -> Result<String, String>;
}
