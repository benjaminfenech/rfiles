use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Local, TimeZone};

static ENGLISH: AtomicBool = AtomicBool::new(false);
static DECIMAL: AtomicBool = AtomicBool::new(false);

/// The date pattern used when none (or an invalid one) is set: "05.11.2026 13:45".
pub const DEFAULT_DATE_PATTERN: &str = "dd.mm.yyyy hh:mm";

/// How dates are shown (Settings → General → Date format): the pattern, turned into chrono's
/// form, and whether recent times read "5 min ago" instead.
struct DateFormat {
    chrono: String,
    relative: bool,
}

static DATE: RwLock<Option<DateFormat>> = RwLock::new(None);

/// Sets the date format from a user pattern (see [`parse_date_pattern`]); an empty or invalid
/// pattern falls back to [`DEFAULT_DATE_PATTERN`]. `relative`: times within the last week read
/// "5 min ago", "2 d ago" in the file list.
pub fn set_date_format(pattern: &str, relative: bool) {
    let chrono = parse_date_pattern(pattern).or_else(|_| parse_date_pattern(DEFAULT_DATE_PATTERN)).unwrap_or_default();
    *DATE.write().unwrap_or_else(|e| e.into_inner()) = Some(DateFormat { chrono, relative });
}

fn date_format() -> (String, bool) {
    match &*DATE.read().unwrap_or_else(|e| e.into_inner()) {
        Some(f) => (f.chrono.clone(), f.relative),
        None => (parse_date_pattern(DEFAULT_DATE_PATTERN).unwrap_or_default(), true),
    }
}

/// A user's date pattern in chrono's form. Fields (upper or lower case): `d`/`dd` day, `m`/`mm`
/// month (`mmm` "Nov", `mmmm` "November"), `yy`/`yyyy` year, `h`/`hh` hour (24 h), `m`/`mm`
/// minutes when right after an hour or right before seconds, `s`/`ss` seconds. Everything that
/// isn't a letter (`: . / -`, spaces, commas) is kept as written. Errors name what isn't a field.
pub fn parse_date_pattern(pattern: &str) -> Result<String, String> {
    #[derive(Clone, Copy, PartialEq)]
    enum Field {
        Day,
        Month,
        Year,
        Hour,
        Minute,
        Second,
    }
    // runs of the same letter, and the text between them
    enum Part {
        Run(char, usize),
        Text(String),
    }
    let mut parts: Vec<Part> = Vec::new();
    for c in pattern.chars() {
        if c.is_alphabetic() {
            let l = c.to_ascii_lowercase();
            match parts.last_mut() {
                Some(Part::Run(p, n)) if *p == l => *n += 1,
                _ => parts.push(Part::Run(l, 1)),
            }
        } else {
            match parts.last_mut() {
                Some(Part::Text(t)) => t.push(c),
                _ => parts.push(Part::Text(c.to_string())),
            }
        }
    }
    // what each run is; `m` is decided once its neighbours are known
    let mut fields: Vec<Option<Field>> = Vec::with_capacity(parts.len());
    for part in &parts {
        fields.push(match part {
            Part::Text(_) => None,
            Part::Run('d', 1..=2) => Some(Field::Day),
            Part::Run('m', 1..=4) => Some(Field::Month),
            Part::Run('y', 2 | 4) => Some(Field::Year),
            Part::Run('h', 1..=2) => Some(Field::Hour),
            Part::Run('s', 1..=2) => Some(Field::Second),
            Part::Run(c, n) => return Err(c.to_string().repeat(*n)),
        });
    }
    let field_at = |i: usize, step: isize| -> Option<Field> {
        let mut j = i as isize + step;
        while j >= 0 && (j as usize) < fields.len() {
            if let Some(f) = fields[j as usize] {
                return Some(f);
            }
            j += step;
        }
        None
    };
    let resolved: Vec<Option<Field>> = (0..fields.len())
        .map(|i| match fields[i] {
            Some(Field::Month) if matches!(&parts[i], Part::Run(_, 1..=2)) && (field_at(i, -1) == Some(Field::Hour) || field_at(i, 1) == Some(Field::Second)) => {
                Some(Field::Minute)
            }
            f => f,
        })
        .collect();
    if resolved.iter().all(Option::is_none) {
        return Err(String::new());
    }
    let mut out = String::new();
    for (part, field) in parts.iter().zip(&resolved) {
        match (part, field) {
            (Part::Text(t), _) => out.push_str(&t.replace('%', "%%")),
            (Part::Run(_, n), Some(f)) => out.push_str(match (f, n) {
                (Field::Day, 1) => "%-d",
                (Field::Day, _) => "%d",
                (Field::Month, 1) => "%-m",
                (Field::Month, 2) => "%m",
                (Field::Month, 3) => "%b",
                (Field::Month, _) => "%B",
                (Field::Year, 2) => "%y",
                (Field::Year, _) => "%Y",
                (Field::Hour, 1) => "%-H",
                (Field::Hour, _) => "%H",
                (Field::Minute, 1) => "%-M",
                (Field::Minute, _) => "%M",
                (Field::Second, 1) => "%-S",
                (Field::Second, _) => "%S",
            }),
            (Part::Run(..), None) => {}
        }
    }
    Ok(out)
}

/// What `pattern` gives for `ms` (Settings shows it as a preview); `None` when the pattern
/// isn't valid.
pub fn preview_date_pattern(pattern: &str, ms: i64) -> Option<String> {
    let chrono = parse_date_pattern(pattern).ok()?;
    let chrono::LocalResult::Single(dt) = Local.timestamp_millis_opt(ms) else { return None };
    Some(dt.format(&chrono).to_string())
}

/// Sizes in powers of 1000 instead of 1024 (Settings → Files & folders → Size format).
pub fn set_decimal_sizes(on: bool) {
    DECIMAL.store(on, Ordering::Relaxed);
}

/// UI language switch for the few words in number formats: English uses "bytes" and a decimal
/// point; the default (Turkish) uses "bayt" and a decimal comma.
pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

/// "0 bayt", "512 bayt", "1,2 KB", "34,5 MB", "1,02 GB" (Turkish decimal comma).
pub fn size(bytes: u64) -> String {
    size_with(bytes, ENGLISH.load(Ordering::Relaxed), DECIMAL.load(Ordering::Relaxed))
}

fn size_with(bytes: u64, en: bool, decimal: bool) -> String {
    let base: f64 = if decimal { 1000.0 } else { 1024.0 };
    let units: [&str; 6] = [if en { "bytes" } else { "bayt" }, "KB", "MB", "GB", "TB", "PB"];
    if (bytes as f64) < base {
        return format!("{bytes} {}", units[0]);
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= base && u < units.len() - 1 {
        v /= base;
        u += 1;
    }
    let s = if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    };
    let s = if en { s } else { s.replace('.', ",") };
    format!("{s} {}", units[u])
}

/// A file time in the chosen date format: recent ones as "5 min ago" when that is on.
pub fn datetime(ms: i64) -> String {
    let now = chrono::Local::now().timestamp_millis();
    let (chrono, relative) = date_format();
    datetime_with(ms, &chrono, relative, now, ENGLISH.load(Ordering::Relaxed))
}

/// The full date-time in the chosen date format (Properties, tooltips, cards, About): never
/// relative.
pub fn datetime_full(ms: i64) -> String {
    datetime_with(ms, &date_format().0, false, 0, false)
}

fn datetime_with(ms: i64, chrono: &str, relative: bool, now: i64, en: bool) -> String {
    if ms <= 0 {
        return String::new();
    }
    let chrono::LocalResult::Single(dt) = Local.timestamp_millis_opt(ms) else { return String::new() };
    if relative {
        let secs = (now - ms) / 1000;
        let recent = match secs {
            s if s < 0 => None,
            s if s < 45 => return (if en { "just now" } else { "az önce" }).to_string(),
            s if s < 3600 => Some((((s + 30) / 60).max(1), "dakika önce", "min ago")),
            s if s < 86_400 => Some((s / 3600, "saat önce", "h ago")),
            s if s < 7 * 86_400 => Some((s / 86_400, "gün önce", "d ago")),
            _ => None,
        };
        if let Some((n, tr, enu)) = recent {
            return if en { format!("{n} {enu}") } else { format!("{n} {tr}") };
        }
    }
    dt.format(chrono).to_string()
}

/// Windows FILETIME (100ns since 1601) → Unix ms.
pub fn filetime_to_unix_ms(ft: u64) -> i64 {
    if ft == 0 {
        return 0;
    }
    (ft / 10_000) as i64 - 11_644_473_600_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 bayt");
        assert_eq!(size(1023), "1023 bayt");
        assert_eq!(size(1024), "1,00 KB");
        assert_eq!(size(1536), "1,50 KB");
        assert_eq!(size(10 * 1024 * 1024 + 1), "10,0 MB");
        assert_eq!(size(500 * 1024 * 1024 * 1024), "500 GB");
    }

    #[test]
    fn relative_and_formats() {
        let now = 1_800_000_000_000i64;
        let dotted = parse_date_pattern(DEFAULT_DATE_PATTERN).unwrap();
        assert_eq!(datetime_with(now - 10_000, &dotted, true, now, false), "az önce");
        assert_eq!(datetime_with(now - 5 * 60_000, &dotted, true, now, false), "5 dakika önce");
        assert_eq!(datetime_with(now - 3 * 3_600_000, &dotted, true, now, true), "3 h ago");
        assert_eq!(datetime_with(now - 2 * 86_400_000, &dotted, true, now, false), "2 gün önce");
        assert!(datetime_with(now - 30 * 86_400_000, &dotted, true, now, false).contains('.'));
        // relative off: the pattern even for a minute ago
        let dmy = datetime_with(now - 60_000, &parse_date_pattern("dd/mm/yyyy hh:mm").unwrap(), false, now, false);
        assert_eq!((dmy.len(), &dmy[2..3], &dmy[5..6], &dmy[13..14]), (16, "/", "/", ":"), "{dmy}");
        assert_eq!(size_with(1500, false, true), "1,50 KB");
        assert_eq!(size_with(1500, false, false), "1,46 KB");
    }

    #[test]
    fn date_patterns() {
        let p = |s: &str| parse_date_pattern(s);
        assert_eq!(p("dd/mm/yyyy hh:mm").as_deref(), Ok("%d/%m/%Y %H:%M"));
        assert_eq!(p("DD-MM-YYYY").as_deref(), Ok("%d-%m-%Y"));
        assert_eq!(p("yyyy-mm-dd hh:mm:ss").as_deref(), Ok("%Y-%m-%d %H:%M:%S"));
        assert_eq!(p("dd.mm.yy").as_deref(), Ok("%d.%m.%y"));
        // `mm` is minutes after an hour or before seconds, else the month
        assert_eq!(p("hh:mm dd.mm.yyyy").as_deref(), Ok("%H:%M %d.%m.%Y"));
        assert_eq!(p("mm:ss").as_deref(), Ok("%M:%S"));
        assert_eq!(p("d mmm yyyy, h:mm").as_deref(), Ok("%-d %b %Y, %-H:%M"));
        assert_eq!(p("mmmm").as_deref(), Ok("%B"));
        assert_eq!(p("100% yyyy").as_deref(), Ok("100%% %Y"));
        // not fields
        assert_eq!(p("dd/mm/yyy"), Err("yyy".into()));
        assert_eq!(p("dd q"), Err("q".into()));
        assert_eq!(p(" - "), Err(String::new()));
        assert_eq!(p(""), Err(String::new()));
        // 2026-11-05 13:45:07 local
        let ms = Local.with_ymd_and_hms(2026, 11, 5, 13, 45, 7).unwrap().timestamp_millis();
        assert_eq!(preview_date_pattern("dd/mm/yyyy hh:mm", ms).as_deref(), Some("05/11/2026 13:45"));
        assert_eq!(preview_date_pattern("yyyy-mm-dd hh:mm:ss", ms).as_deref(), Some("2026-11-05 13:45:07"));
        assert_eq!(preview_date_pattern("d.m.yy", ms).as_deref(), Some("5.11.26"));
        assert_eq!(preview_date_pattern("x", ms), None);
    }

    #[test]
    fn filetime() {
        // 2000-01-01T00:00:00Z
        assert_eq!(filetime_to_unix_ms(125_911_584_000_000_000), 946_684_800_000);
        assert_eq!(filetime_to_unix_ms(0), 0);
    }
}
