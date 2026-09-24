//! Распознавание текста (через platform::ocr_recognize) и поиск личных данных.
//! Модель результата и поиск платформенно-независимы.

use regex::Regex;
use std::sync::OnceLock;
use tiny_skia::Rect;

#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    /// x, y, w, h в пикселях исходной картинки.
    pub rect: (f32, f32, f32, f32),
}

#[derive(Clone, Debug, Default)]
pub struct Line {
    pub words: Vec<Word>,
}

impl Line {
    /// Текст строки и байтовые диапазоны слов в нём (слова через пробел).
    fn text_with_spans(&self) -> (String, Vec<(usize, usize)>) {
        let mut text = String::new();
        let mut spans = Vec::with_capacity(self.words.len());
        for (i, w) in self.words.iter().enumerate() {
            if i > 0 {
                text.push(' ');
            }
            let start = text.len();
            text.push_str(&w.text);
            spans.push((start, text.len()));
        }
        (text, spans)
    }
}

/// Распознанный текст построчно.
pub fn text_of(lines: &[Line]) -> String {
    lines.iter().map(|l| l.text_with_spans().0).collect::<Vec<_>>().join("\n")
}

/// Что именно нашли (для подписи «Скрыто: …»).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Email,
    Phone,
    Card,
    /// Длинный номер: счёт, договор, документ (12+ цифр), СНИЛС, паспорт.
    Number,
    Secret,
    Ip,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Email => "почта",
            Kind::Phone => "телефон",
            Kind::Card => "карта",
            Kind::Number => "номер",
            Kind::Secret => "ключ/пароль",
            Kind::Ip => "IP",
        }
    }
}

struct Patterns {
    list: Vec<(Kind, Regex)>,
    /// «пароль: значение» — скрывается только значение (группа 1).
    labeled: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let r = |s: &str| Regex::new(s).expect("valid regex");
        Patterns {
            list: vec![
                (Kind::Email, r(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)*\.[A-Za-z]{2,}")),
                (Kind::Secret, r(r"eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{4,}")),
                // Ключи с известным префиксом; OCR иногда вставляет пробелы внутрь, поэтому
                // продолжение через пробел тоже берём (лишний раз скрыть безопаснее).
                (
                    Kind::Secret,
                    r(r"\b(?:sk-|gh[pousr]_|github_pat_|AKIA|xox[abprs]-|AIza|glpat-)[A-Za-z0-9_\-]*(?: [A-Za-z0-9_\-]{2,})*"),
                ),
                // Длинная строка из букв разного регистра и цифр: токены, ключи, хэши паролей.
                (Kind::Secret, r(r"\b[A-Za-z0-9_\-+/=]{24,}\b")),
                // Длинное число целиком (группы через пробел или дефис). Карта по Луну
                // помечается как карта, остальное как номер: счёт (20 цифр), договор и т.п.
                (Kind::Number, r(r"\b\d(?:[ \-]?\d){11,}\b")),
                // СНИЛС и паспорт РФ в привычной записи.
                (Kind::Number, r(r"\b\d{3}-\d{3}-\d{3}[ \-]\d{2}\b|\b\d{2} \d{2} \d{6}\b")),
                (
                    Kind::Phone,
                    r(r"(?:\+\d{1,3}|\b8)[\s\-]?\(?\d{3}\)?[\s\-]?\d{3}[\s\-]?\d{2}[\s\-]?\d{2}\b|\+\d[\d\s\-()]{8,16}\d"),
                ),
                (Kind::Ip, r(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")),
            ],
            labeled: r(r"(?i)(?:парол[ья]|password|passwd|pwd|pass|token|токен|secret|секрет|api[_ ]?key|ключ)\s*[:=]\s*(\S+)"),
        }
    })
}

fn luhn(digits: &str) -> bool {
    let ds: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if ds.len() < 13 || ds.len() > 19 {
        return false;
    }
    let sum: u32 = ds
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| if i % 2 == 1 { let x = d * 2; if x > 9 { x - 9 } else { x } } else { d })
        .sum();
    sum % 10 == 0
}

fn accept(kind: Kind, s: &str, pattern_index: usize) -> bool {
    match kind {
        Kind::Card => luhn(s),
        Kind::Ip => s.split('.').all(|p| p.parse::<u16>().is_ok_and(|v| v <= 255)),
        // Общий «длинный токен» (третий шаблон): нужны и буквы обоих регистров, и цифры,
        // иначе это длинное слово или путь, а не ключ.
        Kind::Secret if pattern_index == 2 => s.chars().filter(|c| !c.is_whitespace()).count() >= 16,
        Kind::Secret if pattern_index == 3 => {
            s.chars().any(|c| c.is_ascii_lowercase()) && s.chars().any(|c| c.is_ascii_uppercase()) && s.chars().any(|c| c.is_ascii_digit())
        }
        _ => true,
    }
}

fn union(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
    let (x1, y1) = ((a.0 + a.2).max(b.0 + b.2), (a.1 + a.3).max(b.1 + b.3));
    (x0, y0, x1 - x0, y1 - y0)
}

/// Прямоугольники слов, попавших в байтовый диапазон [s, e) строки.
fn rect_for(line: &Line, spans: &[(usize, usize)], s: usize, e: usize) -> Option<(f32, f32, f32, f32)> {
    let mut acc: Option<(f32, f32, f32, f32)> = None;
    for (w, &(ws, we)) in line.words.iter().zip(spans) {
        if ws < e && s < we {
            acc = Some(acc.map_or(w.rect, |a| union(a, w.rect)));
        }
    }
    acc
}

/// Найти личные данные. Возвращает вид и прямоугольник (с запасом pad) в пикселях картинки.
pub fn find_sensitive(lines: &[Line], pad: f32) -> Vec<(Kind, Rect)> {
    let p = patterns();
    let mut out: Vec<(Kind, (f32, f32, f32, f32))> = Vec::new();
    for line in lines {
        let (text, spans) = line.text_with_spans();
        let mut taken: Vec<(usize, usize)> = Vec::new();
        let overlaps = |taken: &[(usize, usize)], s: usize, e: usize| taken.iter().any(|&(a, b)| a < e && s < b);
        if let Some(c) = p.labeled.captures(&text) {
            if let Some(m) = c.get(1) {
                if let Some(r) = rect_for(line, &spans, m.start(), m.end()) {
                    out.push((Kind::Secret, r));
                    taken.push((m.start(), m.end()));
                }
            }
        }
        for (i, (kind, re)) in p.list.iter().enumerate() {
            for m in re.find_iter(&text) {
                if overlaps(&taken, m.start(), m.end()) || !accept(*kind, m.as_str(), i) {
                    continue;
                }
                let kind = if *kind == Kind::Number && luhn(m.as_str()) { Kind::Card } else { *kind };
                if let Some(r) = rect_for(line, &spans, m.start(), m.end()) {
                    out.push((kind, r));
                    taken.push((m.start(), m.end()));
                }
            }
        }
    }
    out.into_iter()
        .filter_map(|(k, (x, y, w, h))| Rect::from_xywh(x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad).map(|r| (k, r)))
        .collect()
}

/// «Скрыто: 3 (почта 1, телефон 2)».
pub fn summary(found: &[(Kind, Rect)]) -> String {
    if found.is_empty() {
        return "Личных данных не найдено".into();
    }
    let mut counts: std::collections::BTreeMap<Kind, usize> = Default::default();
    for (k, _) in found {
        *counts.entry(*k).or_default() += 1;
    }
    let parts: Vec<String> = counts.iter().map(|(k, n)| format!("{} {n}", k.label())).collect();
    format!("Скрыто: {} ({})", found.len(), parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(words: &[&str]) -> Line {
        let mut x = 0.0;
        Line {
            words: words
                .iter()
                .map(|t| {
                    let w = Word { text: t.to_string(), rect: (x, 0.0, t.len() as f32 * 8.0, 16.0) };
                    x += t.len() as f32 * 8.0 + 8.0;
                    w
                })
                .collect(),
        }
    }

    #[test]
    fn finds_common_data() {
        let lines = vec![
            line(&["Почта:", "ivan.petrov@mail.ru", "тел", "+7", "(912)", "345-67-89"]),
            line(&["Карта", "4111", "1111", "1111", "1111", "и", "не", "карта", "1234", "5678", "9012", "3456"]),
            line(&["пароль:", "Qwerty123", "ip", "192.168.1.10", "и", "999.1.1.1"]),
            line(&["key", "sk-proj-AbCdEf0123456789XyZ"]),
        ];
        let f = find_sensitive(&lines, 0.0);
        let kinds: Vec<Kind> = f.iter().map(|(k, _)| *k).collect();
        assert!(kinds.contains(&Kind::Email));
        assert!(kinds.contains(&Kind::Phone));
        assert_eq!(kinds.iter().filter(|k| **k == Kind::Card).count(), 1, "{kinds:?}");
        // «1234 5678 9012 3456» не карта по Луну, но длинный номер: тоже скрывается.
        assert_eq!(kinds.iter().filter(|k| **k == Kind::Number).count(), 1, "{kinds:?}");
        assert_eq!(kinds.iter().filter(|k| **k == Kind::Ip).count(), 1);
        assert_eq!(kinds.iter().filter(|k| **k == Kind::Secret).count(), 2);
        // Телефон из трёх слов: прямоугольник охватывает их все.
        let phone = f.iter().find(|(k, _)| *k == Kind::Phone).unwrap().1;
        assert!(phone.width() > 3.0 * 16.0);
    }

    #[test]
    fn long_numbers_and_documents() {
        let lines = vec![
            line(&["Счёт", "4257", "1111", "2255", "6888", "4555"]),
            line(&["р/с", "40817810099910004312"]),
            line(&["СНИЛС", "112-233-445", "95", "паспорт", "45", "06", "123456"]),
        ];
        let f = find_sensitive(&lines, 0.0);
        assert_eq!(f.len(), 4, "{f:?}");
        assert!(f.iter().all(|(k, _)| *k == Kind::Number));
        // Весь 20-значный номер одним прямоугольником.
        assert!(f[0].1.width() > 5.0 * 32.0);
    }

    #[test]
    fn ordinary_text_is_clean() {
        let lines = vec![line(&["Отчёт", "за", "сентябрь", "2026", "года,", "версия", "1.2.3"])];
        assert!(find_sensitive(&lines, 0.0).is_empty());
    }
}
