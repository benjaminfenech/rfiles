//! Percent-encoding (`%XX`), `file://` URIs and hex strings, shared by everything that reads or
//! writes them: trash info files, the clipboard, file dialogs, thumbnails, network addresses,
//! web searches, history.

use std::path::PathBuf;

/// Escapes every byte except ASCII letters, digits and those in `keep` as `%XX` (upper case).
/// Each caller keeps its own set: a trash file, a GLib thumbnail hash and a URL's user part
/// each have their own rules.
pub fn encode(bytes: &[u8], keep: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || keep.contains(&b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX_UPPER[(b >> 4) as usize] as char);
            out.push(HEX_UPPER[(b & 15) as usize] as char);
        }
    }
    out
}

/// `%XX` escapes to bytes; a `%` not followed by two hex digits stays as it is.
pub fn decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// [`decode`] as text (invalid UTF-8 replaced).
pub fn decode_str(s: &str) -> String {
    String::from_utf8_lossy(&decode(s)).into_owned()
}

/// [`decode`] as a path: any bytes, like Linux file names.
#[cfg(unix)]
pub fn decode_path(s: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(decode(s)))
}

/// `file:///a%20b` or `file://localhost/a%20b` → `/a b`. Other schemes and other hosts: None.
#[cfg(unix)]
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.trim().strip_prefix("file://")?;
    let rest = if rest.starts_with('/') { rest } else { rest.strip_prefix("localhost")? };
    rest.starts_with('/').then(|| decode_path(rest))
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// Lower-case hex, as digests are written (`sha256sum`, GLib's thumbnail names).
pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let p = "/home/me/a b/ğ%#?.txt";
        let e = encode(p.as_bytes(), b"/-._~");
        assert_eq!(e, "/home/me/a%20b/%C4%9F%25%23%3F.txt");
        assert_eq!(decode_str(&e), p);
        assert_eq!(decode_str("100%"), "100%");
        assert_eq!(decode_str("%zz%4"), "%zz%4");
        assert_eq!(decode(&[0xff].iter().map(|b| format!("%{b:02X}")).collect::<String>()), [0xff]);
    }

    #[test]
    fn file_uris() {
        assert_eq!(file_uri_to_path("file:///a%20b"), Some(PathBuf::from("/a b")));
        assert_eq!(file_uri_to_path("file://localhost/x"), Some(PathBuf::from("/x")));
        assert_eq!(file_uri_to_path(" file:///x\r"), Some(PathBuf::from("/x")));
        assert_eq!(file_uri_to_path("file://nas/x"), None);
        assert_eq!(file_uri_to_path("smb://nas/x"), None);
    }

    #[test]
    fn hex_digits() {
        assert_eq!(hex(&[0, 0xab, 0x10]), "00ab10");
    }
}
