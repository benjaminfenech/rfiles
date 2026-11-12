//! UI language. The English text is the key: `t!("New folder")` returns that text in the active
//! language, from `locales/<code>.json` (embedded at build time, see build.rs). Turkish is one of
//! those files (`tr.json`), like every other language; English needs none.
//!
//! Everything is looked up at render time (nothing is cached at startup), so switching the
//! language only needs a window refresh. Strings produced elsewhere (error messages from the
//! lower crates, date-group labels from a worker thread, tag names) are translated where they
//! are drawn, via [`td`].
//!
//! A locale file maps ids to text: the id of a key is its FNV-1a 32 hash in hex
//! (`locales/_source.json` lists every key with its id; `scripts/i18n_check.py` checks files).
//! A missing text falls back to English.
//!
//! Texts with values in them name their placeholders, which are filled after the lookup, so a
//! translation can move them: `t!("Connecting to {server}…", server = name)`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{LazyLock, OnceLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// Every key with its id and English text (the list translators work from).
const SOURCE: &str = include_str!("../locales/_source.json");

/// The language setting. `Other(i)` is `LOCALES[i]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    /// Follow the desktop's language (`LANGUAGE` / `LC_ALL` / `LC_MESSAGES` / `LANG`).
    #[default]
    System,
    En,
    Other(u16),
}

impl Lang {
    /// "system", "en", or a locale code ("tr", "de-DE"), as stored in the settings file.
    pub fn code(self) -> &'static str {
        match self {
            Lang::System => "system",
            Lang::En => "en",
            Lang::Other(i) => LOCALES.get(i as usize).map_or("en", |l| l.0),
        }
    }

    pub fn from_code(code: &str) -> Lang {
        match code {
            "system" => Lang::System,
            "en" | "en-US" => Lang::En,
            "tr-TR" => Lang::from_code("tr"),
            c => locale(c).unwrap_or(Lang::System),
        }
    }

    /// `System` resolved to a concrete language.
    pub fn resolve(self) -> Lang {
        match self {
            Lang::System => system_lang(),
            other => other,
        }
    }

    /// The language's own name for itself ("Deutsch", "日本語").
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::System => "",
            Lang::En => "English",
            Lang::Other(_) => {
                let code = self.code();
                NATIVE.iter().find(|n| n.0 == code).map_or(code, |n| n.1)
            }
        }
    }

    /// Every language offered: English, then the locale files, by name. Right-to-left
    /// languages are left out until GPUI lays text out right to left (they draw reversed now).
    pub fn all() -> Vec<Lang> {
        let mut others: Vec<Lang> = (0..LOCALES.len() as u16).map(Lang::Other).filter(|l| !l.is_rtl()).collect();
        others.sort_by_key(|l| l.native_name().to_lowercase());
        std::iter::once(Lang::En).chain(others).collect()
    }

    fn is_rtl(self) -> bool {
        RTL.contains(&self.code())
    }
}

/// The locale file with this code, ignoring case.
fn locale(code: &str) -> Option<Lang> {
    LOCALES.iter().position(|l| l.0.eq_ignore_ascii_case(code)).map(|i| Lang::Other(i as u16))
}

/// Written right to left; translated, but not offered yet (see [`Lang::all`]).
const RTL: &[&str] = &["ar", "he-IL", "fa-IR"];

impl Serialize for Lang {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.code())
    }
}

impl<'de> Deserialize<'de> for Lang {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Lang::from_code(&String::deserialize(d)?))
    }
}

/// Names in their own language, for every locale rfiles ships (shown when a file exists).
const NATIVE: &[(&str, &str)] = &[
    ("af", "Afrikaans"),
    ("ar", "العربية"),
    ("be-BY", "Беларуская"),
    ("bg", "Български"),
    ("ca", "Català"),
    ("cs-CZ", "Čeština"),
    ("da", "Dansk"),
    ("de-DE", "Deutsch"),
    ("el", "Ελληνικά"),
    ("en-GB", "English (United Kingdom)"),
    ("es-419", "Español (Latinoamérica)"),
    ("es-ES", "Español (España)"),
    ("fa-IR", "فارسی"),
    ("fi-FI", "Suomi"),
    ("fil-PH", "Filipino"),
    ("fr-FR", "Français"), // i18n-ok
    ("he-IL", "עברית"),
    ("hi-IN", "हिन्दी"),
    ("hr-HR", "Hrvatski"),
    ("hu-HU", "Magyar"),
    ("hy-AM", "Հայերեն"),
    ("id-ID", "Bahasa Indonesia"),
    ("it-IT", "Italiano"),
    ("ja-JP", "日本語"),
    ("ka", "ქართული"),
    ("km-KH", "ខ្មែរ"),
    ("ko-KR", "한국어"),
    ("lt-LT", "Lietuvių"),
    ("lv-LV", "Latviešu"),
    ("ms-MY", "Bahasa Melayu"),
    ("nb-NO", "Norsk bokmål"),
    ("nl-NL", "Nederlands"),
    ("pl-PL", "Polski"),
    ("pt-BR", "Português (Brasil)"),
    ("pt-PT", "Português (Portugal)"),
    ("ro-RO", "Română"),
    ("ru-RU", "Русский"),
    ("sk-SK", "Slovenčina"),
    ("sq-AL", "Shqip"),
    ("sr-Cyrl", "Српски"),
    ("sv-SE", "Svenska"),
    ("ta", "தமிழ்"),
    ("th-TH", "ไทย"),
    ("tr", "Türkçe"), // i18n-ok
    ("uk-UA", "Українська"),
    ("vi", "Tiếng Việt"),
    ("zh-Hans", "简体中文"),
    ("zh-Hant", "繁體中文"),
];

/// Picks the translation for a language tag ("de-AT", "zh-TW", "sr-Cyrl-RS").
fn match_tag(tag: &str) -> Lang {
    let tag = tag.trim();
    let lower = tag.to_ascii_lowercase();
    let lang = lower.split('-').next().unwrap_or("");
    match lang {
        "en" if lower != "en-gb" => return Lang::En,
        // right to left: English until GPUI can draw it
        "ar" | "he" | "iw" | "fa" => return Lang::En,
        _ => {}
    }
    // exact tag first ("pt-BR", "en-GB", "es-ES")
    if let Some(l) = locale(tag) {
        return l;
    }
    let special = match lang {
        "zh" => Some(if ["-tw", "-hk", "-mo", "-hant"].iter().any(|s| lower.contains(s)) { "zh-Hant" } else { "zh-Hans" }),
        "sr" => Some("sr-Cyrl"),
        "no" | "nb" | "nn" => Some("nb-NO"),
        "es" => Some(if lower == "es-es" { "es-ES" } else { "es-419" }),
        "pt" => Some(if lower == "pt-br" { "pt-BR" } else { "pt-PT" }),
        "en" => Some("en-GB"),
        _ => None,
    };
    if let Some(l) = special.and_then(locale) {
        return l;
    }
    // same language, any region ("de-AT" → "de-DE", "tr-TR" → "tr")
    LOCALES
        .iter()
        .position(|l| l.0.split('-').next().is_some_and(|x| x.eq_ignore_ascii_case(lang)))
        .map_or(Lang::En, |i| Lang::Other(i as u16))
}

/// The desktop language's best translation (English when there is none).
fn system_lang() -> Lang {
    match crate::sys::ui_language_tag() {
        Some(tag) => match_tag(&tag),
        None => Lang::En,
    }
}

// 0 = En, 1 + i = LOCALES[i] (resolved; never System).
static ACTIVE: AtomicU16 = AtomicU16::new(0);

/// Sets the UI language (resolving `System`), and the decimal comma and the few words in
/// rf-core's number and date formats.
pub fn set(lang: Lang) {
    let l = lang.resolve();
    ACTIVE.store(
        match l {
            Lang::En | Lang::System => 0,
            Lang::Other(i) => 1 + i,
        },
        Ordering::Relaxed,
    );
    rf_core::format::set_english(!is_tr());
    rf_core::format::set_translator(t);
}

/// The active (resolved) language.
pub fn active() -> Lang {
    match ACTIVE.load(Ordering::Relaxed) {
        0 => Lang::En,
        n => Lang::Other(n - 1),
    }
}

/// Turkish: number formats with a decimal comma and "." between thousands.
pub fn is_tr() -> bool {
    active().code() == "tr"
}

/// Parsed locale files, each on first use. Texts are leaked once (a few thousand per language).
static TABLES: LazyLock<Vec<OnceLock<HashMap<u32, &'static str>>>> = LazyLock::new(|| LOCALES.iter().map(|_| OnceLock::new()).collect());

fn table(i: u16) -> Option<&'static HashMap<u32, &'static str>> {
    let cell = TABLES.get(i as usize)?;
    Some(cell.get_or_init(|| {
        let raw: HashMap<String, String> = serde_json::from_str(LOCALES[i as usize].1).unwrap_or_else(|e| {
            log::warn!("i18n: {} unreadable: {e}", LOCALES[i as usize].0);
            HashMap::new()
        });
        raw.into_iter()
            .filter_map(|(k, v)| Some((u32::from_str_radix(&k, 16).ok()?, &*Box::leak(v.into_boxed_str()))))
            .collect()
    }))
}

/// The id of a key: FNV-1a 32 over its UTF-8 bytes (same as the tools in scripts/).
pub fn key_id(key: &str) -> u32 {
    key.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// `key` in the active language, None in English or when the language has no text for it.
fn lookup(key: &str) -> Option<&'static str> {
    match active() {
        Lang::En | Lang::System => None,
        Lang::Other(i) => table(i).and_then(|t| t.get(&key_id(key)).copied()),
    }
}

/// Translates an English literal. A text missing in the active language stays English.
pub fn t(en: &'static str) -> &'static str {
    lookup(en).unwrap_or(en)
}

/// Translates a templated text, then fills its `{name}` placeholders. Used by `t!` with values.
pub fn t_fill(en: &'static str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    fill(t(en), |name| values.iter().find(|v| v.0 == name).map(|v| v.1.to_string()))
}

/// `text` with every `{name}` that `value` knows replaced; other braces stay as they are.
fn fill(text: &str, value: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out += &rest[..open];
        let after = &rest[open + 1..];
        match after.find('}').and_then(|close| Some((close, value(&after[..close])?))) {
            Some((close, v)) => {
                out += &v;
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out + rest
}

/// One `_source.json` entry; only what [`td`] needs.
#[derive(Deserialize)]
struct SourceEntry {
    en: String,
    #[serde(default)]
    key: Option<String>,
}

/// The keys with placeholders, split at them: "Couldn't read: {}" → (["Couldn't read: ", ""],
/// ["{}"]). [`td`] matches finished messages against them.
static TEMPLATES: LazyLock<Vec<(&'static str, Vec<&'static str>, Vec<&'static str>)>> = LazyLock::new(|| {
    let src: Vec<SourceEntry> = serde_json::from_str(SOURCE).unwrap_or_default();
    src.into_iter()
        .filter(|e| e.key.is_none() && e.en.contains('{'))
        .filter_map(|e| {
            let en: &'static str = Box::leak(e.en.into_boxed_str());
            let (mut parts, mut names) = (vec![], vec![]);
            let mut rest = en;
            while let Some(open) = rest.find('{') {
                let close = open + rest[open..].find('}')?;
                parts.push(&rest[..open]);
                names.push(&rest[open..=close]);
                rest = &rest[close + 1..];
            }
            parts.push(rest);
            // a template that is all placeholders ("{e:#}") would match anything
            (parts.iter().any(|p| p.len() > 2)).then_some((en, parts, names))
        })
        .collect()
});

/// Matches `s` against `parts` (the literal pieces of a template) and returns what stood where
/// the placeholders were: "Couldn't read: /x" against ["Couldn't read: ", ""] → ["/x"].
fn match_template<'a>(s: &'a str, parts: &[&str]) -> Option<Vec<&'a str>> {
    let mut rest = s.strip_prefix(parts[0])?;
    let mut caps = vec![];
    for (i, part) in parts.iter().enumerate().skip(1) {
        let at = if i == parts.len() - 1 {
            // the last piece ends the text
            rest.strip_suffix(part).map(|r| r.len())?
        } else if part.is_empty() {
            return None; // two placeholders in a row can't be split
        } else {
            rest.find(part)?
        };
        caps.push(&rest[..at]);
        rest = &rest[at + part.len()..];
    }
    Some(caps)
}

/// English texts by their Turkish wording, for strings saved before English became the source
/// (tag names, labels in history files) — they show in the active language like new ones.
static FROM_TURKISH: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let Some(tr) = locale("tr").and_then(|l| if let Lang::Other(i) = l { table(i) } else { None }) else { return HashMap::new() };
    let src: Vec<SourceEntry> = serde_json::from_str(SOURCE).unwrap_or_default();
    let keys: std::collections::HashSet<&str> = src.iter().map(|e| e.en.as_str()).collect();
    let map: HashMap<&'static str, &'static str> = src
        .iter()
        .filter(|e| e.key.is_none())
        .filter_map(|e| Some((*tr.get(&key_id(&e.en))?, &*Box::leak(e.en.clone().into_boxed_str()))))
        // a Turkish text that is also an English key ("OK", "ZIP") is that key
        .filter(|(tr, _)| !keys.contains(tr))
        .collect();
    map
});

/// Translates a string made outside `t!`: an error message from a lower crate, a group label,
/// a tag name. In order: the whole text as a key; a key with placeholders that it fills
/// ("Couldn't read: {}" for "Couldn't read: /x", the values kept as they are); the part before
/// the first ": " ("Folder not found: /x"). A text saved in Turkish before English became the
/// source is read as its English key. Anything else is returned unchanged.
pub fn td(s: &str) -> String {
    let s = FROM_TURKISH.get(s).copied().unwrap_or(s);
    if let Some(t) = lookup(s) {
        return t.to_string();
    }
    if active() == Lang::En {
        return s.to_string();
    }
    for (en, parts, names) in TEMPLATES.iter() {
        if let (Some(caps), Some(tr)) = (match_template(s, parts), lookup(en)) {
            // the same placeholder twice is filled by its first value
            return fill(tr, |name| names.iter().position(|n| n[1..n.len() - 1] == *name).map(|i| caps[i].to_string()));
        }
    }
    if let Some((head, rest)) = s.split_once(": ") {
        if let Some(t) = lookup(head) {
            return format!("{t}: {rest}");
        }
    }
    s.to_string()
}

/// A counted / templated text keyed by name ("@items_one"), with its English.
fn keyed(key: &str, en: &'static str) -> &'static str {
    lookup(key).unwrap_or(en)
}

/// Percentage in the UI language's order: English "53%", Turkish "%53", French "53 %".
pub fn pct(v: f32) -> String {
    keyed("@percent", "{v}%").replace("{v}", &format!("{v:.0}"))
}

fn counted(n: usize, one: &str, other: &str, en_one: &'static str, en_other: &'static str) -> String {
    let text = if n == 1 { keyed(one, en_one) } else { keyed(other, en_other) };
    text.replace("{n}", &n.to_string())
}

/// "1 item" / "5 items".
pub fn items(n: usize) -> String {
    counted(n, "@items_one", "@items_other", "{n} item", "{n} items")
}

/// "3 items selected".
pub fn items_selected(n: usize) -> String {
    counted(n, "@selected_one", "@selected_other", "{n} item selected", "{n} items selected")
}

/// `t!("English")` → the text in the UI language (`&'static str`).
/// `t!("Copying {name}…", name = x)` → the same, with `{name}` filled after the lookup (`String`).
#[macro_export]
macro_rules! t {
    ($s:literal) => {
        $crate::i18n::t($s)
    };
    ($s:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::t_fill($s, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr() -> Lang {
        locale("tr").expect("locales/tr.json")
    }

    #[test]
    fn ids_match_the_tools() {
        // python: fnv1a32("Aranacak metin") in scripts → 29b4f21c
        assert_eq!(format!("{:08x}", key_id("Aranacak metin")), "29b4f21c");
    }

    #[test]
    fn source_lists_unique_ids() {
        let src: Vec<serde_json::Value> = serde_json::from_str(SOURCE).unwrap();
        let mut seen = std::collections::HashSet::new();
        for e in &src {
            let key = e.get("key").or_else(|| e.get("en")).and_then(|v| v.as_str()).unwrap();
            let id = e["id"].as_str().unwrap();
            assert!(seen.insert(id.to_string()), "duplicate id {id}");
            if !key.starts_with('@') {
                assert_eq!(format!("{:08x}", key_id(key)), id, "id of {key:?}");
            }
        }
    }

    #[test]
    fn codes_round_trip_and_tags_match() {
        assert_eq!(Lang::from_code("tr"), tr());
        assert_eq!(Lang::from_code("tr-TR"), tr());
        assert_eq!(Lang::from_code("en"), Lang::En);
        assert_eq!(Lang::from_code("system"), Lang::System);
        assert_eq!(Lang::from_code("xx-YY"), Lang::System);
        assert_eq!(match_tag("tr-TR"), tr());
        assert_eq!(match_tag("en-US"), Lang::En);
        assert_eq!(Lang::all()[0], Lang::En);
        assert!(Lang::all().contains(&tr()));
        assert_eq!(tr().native_name(), "Türkçe");
        for l in Lang::all() {
            assert_eq!(Lang::from_code(l.code()), l);
            assert!(!l.native_name().is_empty());
            assert!(!l.is_rtl());
        }
        assert_eq!(match_tag("ar-SA"), Lang::En);
        assert_eq!(match_tag("de-AT"), locale("de-DE").unwrap());
        assert_eq!(match_tag("zh-TW"), locale("zh-Hant").unwrap());
        assert_eq!(match_tag("es-MX"), locale("es-419").unwrap());
    }

    #[test]
    fn fill_and_templates() {
        assert_eq!(fill("{a} and {b} {c}", |n| (n != "c").then(|| n.to_uppercase())), "A and B {c}");
        assert_eq!(fill("{n:03} {", |_| None), "{n:03} {");
        assert_eq!(match_template("Couldn't read: /x", &["Couldn't read: ", ""]), Some(vec!["/x"]));
        assert_eq!(match_template("Branch 'dev' doesn't track", &["Branch '", "' doesn't track"]), Some(vec!["dev"]));
        assert_eq!(match_template("Something else", &["Couldn't read: ", ""]), None);
    }

    #[test]
    fn languages_translate() {
        // One test mutates the global language; keep everything in it.
        set(Lang::En);
        assert_eq!(t("New folder"), "New folder");
        assert_eq!(t!("{root} {n} items, {mb} MB", root = "/home", n = 5, mb = 12), "/home 5 items, 12 MB");
        assert_eq!(items(1), "1 item");
        assert_eq!(items(3), "3 items");
        assert_eq!(items_selected(2), "2 items selected");
        assert_eq!(pct(53.), "53%");
        assert_eq!(td("Folder not found: /x"), "Folder not found: /x");
        // saved in Turkish by an older version: shown as its English key
        assert_eq!(td("Kırmızı"), "Red");
        assert_eq!(rf_core::format::size(1536), "1.50 KB");

        set(tr());
        assert_eq!(t("New folder"), "Yeni klasör");
        assert_eq!(t("Trash"), "Çöp Kutusu");
        assert_eq!(t("Light"), "Açık");
        assert_eq!(t!("{root} {n} items, {mb} MB", root = "/home", n = 5, mb = 12), "/home 5 öğe, 12 MB");
        assert_eq!(items(1), "1 öğe");
        assert_eq!(pct(53.), "%53");
        assert_eq!(td("Folder not found: /x"), "Klasör bulunamadı: /x");
        assert_eq!(td("Couldn't open the file: /x: denied"), "Dosya açılamadı: /x: denied");
        assert_eq!(td("Branch 'dev' doesn't track a remote branch"), "'dev' dalının izlediği bir uzak dal yok");
        assert_eq!(td("Kırmızı"), "Kırmızı");
        assert_eq!(td("kullanıcı dosyası.txt"), "kullanıcı dosyası.txt");
        assert_eq!(rf_core::format::size(1536), "1,50 KB");
        assert_eq!(rf_core::format::size(10), "10 bayt");

        // every locale file: readable, and a missing text falls back to English
        for i in 0..LOCALES.len() as u16 {
            set(Lang::Other(i));
            assert!(!table(i).unwrap().is_empty(), "{} is empty or unreadable", LOCALES[i as usize].0);
            assert!(!t("New folder").is_empty());
            assert_eq!(t("a text no locale has"), "a text no locale has");
            assert!(items(5).contains('5'));
        }
        set(Lang::En);
    }
}
