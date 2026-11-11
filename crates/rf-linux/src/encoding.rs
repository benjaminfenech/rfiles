//! Legacy text encodings. Old Turkish text files are Windows-1254, not UTF-8; decoding them
//! as UTF-8 turns ş/ğ/ı into U+FFFD. Use the user locale's *traditional* ANSI code page (not
//! `GetACP()`, which is 65001 when "Use Unicode UTF-8 for worldwide language support" is on).
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
pub fn legacy_ansi_codepage() -> u32 {
    Default::default()
}
fn decode(_raw: &[u8], _cp: u32) -> Option<String> {
    None
}
/// Text from bytes of unknown encoding: UTF-8 if valid (a truncated trailing sequence is
/// tolerated, since previews read only the head of a file), otherwise the legacy ANSI page.
pub fn decode_text(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(e) if e.error_len().is_none() => {
            String::from_utf8_lossy(&raw[..e.valid_up_to()]).into_owned()
        }
        Err(_) => {
            decode(raw, legacy_ansi_codepage())
                .unwrap_or_else(|| String::from_utf8_lossy(raw).into_owned())
        }
    }
}
/// True when the user's regional format is Turkish (sorting then treats `I` as capital `ı`).
pub fn user_locale_is_turkish() -> bool {
    false
}
