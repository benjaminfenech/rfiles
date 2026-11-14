//! Filters typed into the search box with the name: `size:>10M`, `modified:7d`,
//! `created:2024`, `type:image`. Each filter word is taken out of the query; what is left is the
//! name (or `content:` / `tag:`) search. A word that doesn't parse stays part of the name.
//!
//! * `size:` `>10M`, `>=1.5G`, `<100K`, `10M..1G`, a bare `10M` (at least), or `empty`,
//!   `small` (< 1 MB), `medium` (1–128 MB), `large` (128 MB–1 GB), `huge` (> 1 GB). Units
//!   B, K, M, G, T (powers of 1024). Folders have no size, so a size filter leaves them out.
//! * `modified:` / `created:` (`date:` = `modified:`) `today`, `yesterday`, `7d` / `2w` /
//!   `3m` / `1y` / `12h` (within the last…), `2024`, `2024-05`, `2024-05-01` (that year, month,
//!   day), `>2024-05-01` (after), `<2024-05-01` (before), `2024-01..2024-06` (from, to).
//! * `type:` (`kind:`) `image`, `video`, `audio`, `document`, `spreadsheet`, `pdf`, `text`,
//!   `code`, `archive`, `program`, `folder`, `file`, or an extension (`type:rs`); several
//!   with commas (`type:image,video`) match any of them.
//!
//! Several filters must all match.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use rf_core::Entry;

use crate::icons::Category;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Filters {
    /// Inclusive byte ranges.
    size: Vec<(u64, u64)>,
    /// Half-open ranges of Unix milliseconds.
    modified: Vec<(i64, i64)>,
    created: Vec<(i64, i64)>,
    /// Any of these (one `type:` word); every word must match.
    kinds: Vec<Vec<Kind>>,
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Cat(Category),
    Folder,
    File,
    Ext(String),
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        self.size.is_empty() && self.modified.is_empty() && self.created.is_empty() && self.kinds.is_empty()
    }

    /// Whether `e` passes every filter.
    pub fn keep(&self, e: &Entry) -> bool {
        let in_ms = |ranges: &[(i64, i64)], t: i64| ranges.iter().all(|(a, b)| t != 0 && t >= *a && t < *b);
        (self.size.is_empty() || (!e.is_dir() && self.size.iter().all(|(a, b)| e.size >= *a && e.size <= *b)))
            && in_ms(&self.modified, e.modified_ms)
            && in_ms(&self.created, e.created_ms)
            && self.kinds.iter().all(|any| {
                any.iter().any(|k| match k {
                    Kind::Folder => e.is_dir(),
                    Kind::File => !e.is_dir(),
                    Kind::Cat(c) => crate::icons::category(e) == *c,
                    Kind::Ext(x) => !e.is_dir() && &*e.ext == x.as_str(),
                })
            })
    }
}

/// Takes the filter words out of `query`: (the rest, the filters). `now` is the local time
/// "today" and "7d" count from.
pub fn parse(query: &str, now: DateTime<Local>) -> (String, Filters) {
    let mut f = Filters::default();
    let mut rest: Vec<&str> = Vec::new();
    for word in query.split_whitespace() {
        let Some((key, value)) = word.split_once(':') else {
            rest.push(word);
            continue;
        };
        let ok = match key.to_ascii_lowercase().as_str() {
            "size" => size(value).map(|r| f.size.push(r)).is_some(),
            "modified" | "date" => time(value, now).map(|r| f.modified.push(r)).is_some(),
            "created" => time(value, now).map(|r| f.created.push(r)).is_some(),
            "type" | "kind" => kinds(value).map(|k| f.kinds.push(k)).is_some(),
            _ => false,
        };
        if !ok {
            rest.push(word);
        }
    }
    (rest.join(" "), f)
}

/// `10M`, `1.5g`, `300` (bytes).
fn bytes(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let s = s.strip_suffix('b').unwrap_or(&s);
    let (num, mul) = match s.chars().last()? {
        'k' => (&s[..s.len() - 1], 1u64 << 10),
        'm' => (&s[..s.len() - 1], 1 << 20),
        'g' => (&s[..s.len() - 1], 1 << 30),
        't' => (&s[..s.len() - 1], 1 << 40),
        _ => (s, 1),
    };
    let n: f64 = num.parse().ok().filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    Some((n * mul as f64).round() as u64)
}

fn size(v: &str) -> Option<(u64, u64)> {
    const M: u64 = 1 << 20;
    Some(match v.to_ascii_lowercase().as_str() {
        "empty" => (0, 0),
        "small" => (1, M - 1),
        "medium" => (M, 128 * M - 1),
        "large" => (128 * M, 1024 * M - 1),
        "huge" => (1024 * M, u64::MAX),
        v => {
            if let Some((a, b)) = v.split_once("..") {
                (bytes(a)?, bytes(b)?)
            } else if let Some(x) = v.strip_prefix(">=") {
                (bytes(x)?, u64::MAX)
            } else if let Some(x) = v.strip_prefix("<=") {
                (0, bytes(x)?)
            } else if let Some(x) = v.strip_prefix('>') {
                (bytes(x)?.saturating_add(1), u64::MAX)
            } else if let Some(x) = v.strip_prefix('<') {
                (0, bytes(x)?.checked_sub(1)?)
            } else {
                (bytes(v)?, u64::MAX)
            }
        }
    })
}

fn ms(t: DateTime<Local>) -> i64 {
    t.timestamp_millis()
}

/// Local midnight starting `d`.
fn midnight(d: NaiveDate) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest()
}

/// A day, month or year: `2024-05-01`, `2024-05`, `2024`, as [start, end).
fn period(s: &str) -> Option<(NaiveDate, NaiveDate)> {
    let parts: Vec<&str> = s.split('-').collect();
    let num = |i: usize| parts.get(i)?.parse::<u32>().ok();
    match parts.len() {
        1 if parts[0].len() == 4 => {
            let y = parts[0].parse::<i32>().ok()?;
            Some((NaiveDate::from_ymd_opt(y, 1, 1)?, NaiveDate::from_ymd_opt(y + 1, 1, 1)?))
        }
        2 => {
            let (y, m) = (parts[0].parse::<i32>().ok()?, num(1)?);
            let start = NaiveDate::from_ymd_opt(y, m, 1)?;
            let end = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1)? } else { NaiveDate::from_ymd_opt(y, m + 1, 1)? };
            Some((start, end))
        }
        3 => {
            let d = NaiveDate::from_ymd_opt(parts[0].parse().ok()?, num(1)?, num(2)?)?;
            Some((d, d.succ_opt()?))
        }
        _ => None,
    }
}

fn time(v: &str, now: DateTime<Local>) -> Option<(i64, i64)> {
    let v = v.to_ascii_lowercase();
    let today = now.date_naive();
    let day = |d: NaiveDate| Some((ms(midnight(d)?), ms(midnight(d.succ_opt()?)?)));
    match v.as_str() {
        "today" => return day(today),
        "yesterday" => return day(today.pred_opt()?),
        _ => {}
    }
    // within the last N hours / days / weeks / months / years
    if let Some(unit) = v.chars().last().filter(|c| "hdwmy".contains(*c)) {
        if let Ok(n) = v[..v.len() - 1].parse::<i64>() {
            let from = match unit {
                'h' => now - Duration::hours(n),
                'd' => now - Duration::days(n),
                'w' => now - Duration::weeks(n),
                'm' => now.checked_sub_months(chrono::Months::new(u32::try_from(n).ok()?))?,
                _ => now.with_year(now.year() - i32::try_from(n).ok()?)?,
            };
            return Some((ms(from), i64::MAX));
        }
    }
    if let Some(x) = v.strip_prefix('>') {
        // after that day (month, year)
        return Some((ms(midnight(period(x.trim_start_matches('='))?.1)?), i64::MAX));
    }
    if let Some(x) = v.strip_prefix('<') {
        return Some((i64::MIN, ms(midnight(period(x.trim_start_matches('='))?.0)?)));
    }
    if let Some((a, b)) = v.split_once("..") {
        return Some((ms(midnight(period(a)?.0)?), ms(midnight(period(b)?.1)?)));
    }
    let (a, b) = period(&v)?;
    Some((ms(midnight(a)?), ms(midnight(b)?)))
}

fn kinds(v: &str) -> Option<Vec<Kind>> {
    let out: Vec<Kind> = v
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|w| {
            let w = w.to_lowercase();
            match w.as_str() {
                "image" | "images" | "photo" | "photos" | "picture" | "pictures" => Kind::Cat(Category::Image),
                "video" | "videos" | "movie" | "movies" => Kind::Cat(Category::Video),
                "audio" | "music" | "sound" => Kind::Cat(Category::Audio),
                "document" | "documents" | "doc" | "docs" => Kind::Cat(Category::Document),
                "spreadsheet" | "spreadsheets" | "sheet" => Kind::Cat(Category::Spreadsheet),
                "pdf" => Kind::Cat(Category::Pdf),
                "text" => Kind::Cat(Category::Text),
                "code" | "source" => Kind::Cat(Category::Code),
                "archive" | "archives" | "compressed" => Kind::Cat(Category::Archive),
                "program" | "programs" | "executable" | "app" => Kind::Cat(Category::Executable),
                "folder" | "folders" | "dir" | "directory" => Kind::Folder,
                "file" | "files" => Kind::File,
                ext => Kind::Ext(ext.trim_start_matches('.').to_string()),
            }
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rf_core::EntryKind;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 14, 15, 30, 0).unwrap()
    }

    fn file(name: &str, size: u64, modified: DateTime<Local>) -> Entry {
        Entry::new(name, EntryKind::File, size, modified.timestamp_millis(), modified.timestamp_millis(), 0)
    }

    #[test]
    fn words_come_out_of_the_query() {
        let (rest, f) = parse("holiday size:>10M  type:image,video notes", now());
        assert_eq!(rest, "holiday notes");
        assert_eq!(f.size, [((10 << 20) + 1, u64::MAX)]);
        assert_eq!(f.kinds, [vec![Kind::Cat(Category::Image), Kind::Cat(Category::Video)]]);
        // not filters: an unknown key, a bad value, a plain colon
        let (rest, f) = parse("http://x size:lots tag:red", now());
        assert_eq!(rest, "http://x size:lots tag:red");
        assert!(f.is_empty());
    }

    #[test]
    fn sizes() {
        assert_eq!(size("1.5k"), Some((1536, u64::MAX)));
        assert_eq!(size("<1KB"), Some((0, 1023)));
        assert_eq!(size(">=2m"), Some((2 << 20, u64::MAX)));
        assert_eq!(size("1m..2m"), Some((1 << 20, 2 << 20)));
        assert_eq!(size("empty"), Some((0, 0)));
        assert_eq!(size("<0"), None);
        let (_, f) = parse("size:>1M", now());
        assert!(f.keep(&file("a", 2 << 20, now())));
        assert!(!f.keep(&file("a", 1 << 20, now())));
        assert!(!f.keep(&Entry::new("d", EntryKind::Dir, 0, 0, 0, 0)), "folders have no size");
    }

    #[test]
    fn times() {
        let (_, f) = parse("modified:today", now());
        assert!(f.keep(&file("a", 1, Local.with_ymd_and_hms(2026, 11, 14, 0, 5, 0).unwrap())));
        assert!(!f.keep(&file("a", 1, Local.with_ymd_and_hms(2026, 11, 13, 23, 55, 0).unwrap())));
        let (_, f) = parse("date:7d", now());
        assert!(f.keep(&file("a", 1, Local.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap())));
        assert!(!f.keep(&file("a", 1, Local.with_ymd_and_hms(2026, 11, 6, 12, 0, 0).unwrap())));
        let (_, f) = parse("modified:2024-12", now());
        assert!(f.keep(&file("a", 1, Local.with_ymd_and_hms(2024, 12, 31, 23, 0, 0).unwrap())));
        assert!(!f.keep(&file("a", 1, Local.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap())));
        let (_, f) = parse("created:>2025-06-01", now());
        assert!(!f.keep(&file("a", 1, Local.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap())), "after that day");
        assert!(f.keep(&file("a", 1, Local.with_ymd_and_hms(2025, 6, 2, 0, 0, 0).unwrap())));
        let (_, f) = parse("modified:2023..2024-02", now());
        assert!(f.keep(&file("a", 1, Local.with_ymd_and_hms(2024, 2, 29, 9, 0, 0).unwrap())));
        assert!(!f.keep(&file("a", 1, Local.with_ymd_and_hms(2022, 12, 31, 9, 0, 0).unwrap())));
        assert!(!f.keep(&file("a", 1, Local.with_ymd_and_hms(2024, 3, 1, 0, 0, 0).unwrap())));
        assert!(!parse("modified:today", now()).1.keep(&Entry::new("a", EntryKind::File, 1, 0, 0, 0)), "an unknown time doesn't match");
    }

    #[test]
    fn kinds_and_extensions() {
        let (_, f) = parse("type:folder", now());
        assert!(f.keep(&Entry::new("d", EntryKind::Dir, 0, 0, 0, 0)) && !f.keep(&file("a.png", 1, now())));
        let (_, f) = parse("type:.RS type:file", now());
        assert!(f.keep(&file("main.rs", 1, now())) && !f.keep(&file("main.py", 1, now())));
        let (_, f) = parse("kind:photos", now());
        assert!(f.keep(&file("x.JPG", 1, now())));
    }
}
