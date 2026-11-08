//! ZIP entry names. Entries without the UTF-8 flag are in the creating PC's OEM code page:
//! Explorer's "Compressed (zipped) folder" on Turkish Windows writes CP857, so the `zip` crate's
//! CP437 fallback turns "Şirket" into "×irket". Decode like Explorer does instead.

use std::borrow::Cow;
use std::sync::OnceLock;

#[cfg(windows)]
use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_IDEFAULTCODEPAGE, MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};
#[cfg(windows)]
use windows::core::PCWSTR;

/// The user locale's traditional OEM code page (857 for tr-TR, 437 for en-US). Deliberately
/// not `GetOEMCP()`: with "Use Unicode UTF-8 for worldwide language support" enabled that is
/// 65001, but old archives were still written in the legacy page.
#[cfg(windows)]
pub fn legacy_oem_codepage() -> u32 {
    static CP: OnceLock<u32> = OnceLock::new();
    *CP.get_or_init(|| {
        let mut buf = [0u16; 8];
        let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_IDEFAULTCODEPAGE, Some(&mut buf)) };
        let cp = if n > 1 { String::from_utf16_lossy(&buf[..n as usize - 1]).trim().parse().unwrap_or(437) } else { 437 };
        if cp == 0 || cp == 65001 { 437 } else { cp }
    })
}

#[cfg(windows)]
fn decode_codepage(raw: &[u8], cp: u32) -> Option<String> {
    let n = unsafe { MultiByteToWideChar(cp, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), raw, None) };
    if n <= 0 {
        return None;
    }
    let mut out = vec![0u16; n as usize];
    let n = unsafe { MultiByteToWideChar(cp, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), raw, Some(&mut out)) };
    (n > 0).then(|| String::from_utf16_lossy(&out[..n as usize]))
}

/// Linux has no OEM code page: guess the one Windows would use for the user's language
/// (from `LC_ALL` / `LC_CTYPE` / `LANG`), so zips made on that Windows decode the same way.
/// CJK pages (932, 936, 949, 950) are not covered and fall back to 437.
#[cfg(unix)]
pub fn legacy_oem_codepage() -> u32 {
    static CP: OnceLock<u32> = OnceLock::new();
    *CP.get_or_init(|| {
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
            .unwrap_or_default();
        codepage_for_language(locale.split(['_', '.', '@']).next().unwrap_or(""))
    })
}

#[cfg(unix)]
fn codepage_for_language(lang: &str) -> u32 {
    match lang {
        "tr" | "az" => 857,
        "ru" | "uk" | "be" | "bg" => 866,
        "el" => 737,
        "ar" | "fa" => 720,
        "he" => 862,
        "pl" | "cs" | "sk" | "hu" | "hr" | "sl" | "ro" | "sr" | "bs" | "sq" => 852,
        "lt" | "lv" | "et" => 775,
        "th" => 874,
        "de" | "fr" | "es" | "it" | "nl" | "pt" | "sv" | "da" | "nb" | "nn" | "no" | "fi" | "is" | "ca" | "eu"
        | "gl" | "ga" => 850,
        _ => 437,
    }
}

#[cfg(unix)]
fn decode_codepage(raw: &[u8], cp: u32) -> Option<String> {
    let table = oem_cp::code_table::DECODING_TABLE_CP_MAP.get(&u16::try_from(cp).ok()?)?;
    table.decode_string_checked(raw)
}

/// Best name for an entry: valid UTF-8 bytes are UTF-8 (flagged or not); anything else is the
/// legacy OEM code page; `fallback` (the zip crate's own decoding) if that fails too.
pub fn entry_name<'a>(raw: &[u8], fallback: Cow<'a, str>) -> Cow<'a, str> {
    if raw.is_ascii() {
        return fallback;
    }
    if let Ok(s) = std::str::from_utf8(raw) {
        return Cow::Owned(s.to_string());
    }
    match decode_codepage(raw, legacy_oem_codepage()) {
        Some(s) => Cow::Owned(s),
        None => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp857_names() {
        // "Şirket ığüşöç" in CP857
        let raw = [0x9E, b'i', b'r', b'k', b'e', b't', b' ', 0x8D, 0xA7, 0x81, 0x9F, 0x94, 0x87];
        assert_eq!(decode_codepage(&raw, 857).as_deref(), Some("Şirket ığüşöç"));
        if legacy_oem_codepage() == 857 {
            assert_eq!(entry_name(&raw, Cow::Borrowed("?")), "Şirket ığüşöç");
        }
        assert_eq!(entry_name("Şirket".as_bytes(), Cow::Borrowed("?")), "Şirket");
        assert_eq!(entry_name(b"plain.txt", Cow::Borrowed("plain.txt")), "plain.txt");
    }
}

#[cfg(test)]
mod live {
    /// `RF_ZIP857=<zip without UTF-8 flag, CP857 names>`: listing must show the Turkish names.
    #[test]
    fn lists_cp857_zip() {
        let Ok(p) = std::env::var("RF_ZIP857") else { return };
        let names: Vec<String> = crate::list(std::path::Path::new(&p)).unwrap().into_iter().map(|e| e.path).collect();
        eprintln!("{names:?}");
        assert!(names.iter().any(|n| n == "Şirket raporu.txt"), "{names:?}");
    }
}
