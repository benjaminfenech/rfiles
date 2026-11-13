//! Certificate files (.cer, .crt, .der, .pem): subject, issuer and validity.
//!
//! Linux: rf-win decodes with Windows' CryptoAPI; here a small DER reader walks the X.509
//! structure itself (only the parts shown: the names and the validity dates). PEM files are
//! read from their first `CERTIFICATE` block.

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct CertInfo {
    pub subject: String,
    pub issuer: String,
    /// Unix milliseconds.
    pub not_before: i64,
    pub not_after: i64,
}

pub fn info(path: &Path) -> Option<CertInfo> {
    let data = std::fs::read(path).ok()?;
    if data.len() > 1 << 20 {
        return None;
    }
    let der = if data.starts_with(&[0x30]) { data } else { pem_block(&String::from_utf8_lossy(&data))? };
    parse(&der)
}

/// The first `-----BEGIN CERTIFICATE-----` block, base64-decoded.
fn pem_block(text: &str) -> Option<Vec<u8>> {
    let start = text.find("-----BEGIN CERTIFICATE-----")? + "-----BEGIN CERTIFICATE-----".len();
    let end = start + text[start..].find("-----END CERTIFICATE-----")?;
    base64(&text[start..end])
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// One DER element: (tag, contents, the rest after it).
fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *b.first()?;
    let first = *b.get(1)?;
    let (len, head) = if first < 0x80 {
        (first as usize, 2)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *b.get(2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let body = b.get(head..head + len)?;
    Some((tag, body, &b[head + len..]))
}

fn parse(der: &[u8]) -> Option<CertInfo> {
    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signature }
    let (0x30, cert, _) = tlv(der)? else { return None };
    let (0x30, tbs, _) = tlv(cert)? else { return None };
    let mut rest = tbs;
    // [0] version (optional)
    if rest.first() == Some(&0xa0) {
        rest = tlv(rest)?.2;
    }
    let (_, _serial, r) = tlv(rest)?;
    let (_, _sig_alg, r) = tlv(r)?;
    let (0x30, issuer, r) = tlv(r)? else { return None };
    let (0x30, validity, r) = tlv(r)? else { return None };
    let (0x30, subject, _) = tlv(r)? else { return None };
    let (t1, nb, v) = tlv(validity)?;
    let (t2, na, _) = tlv(v)?;
    Some(CertInfo { subject: display_name(subject), issuer: display_name(issuer), not_before: time(t1, nb)?, not_after: time(t2, na)? })
}

/// A Name's attributes as (short name, value), in order.
fn attributes(name: &[u8]) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut sets = name;
    while let Some((_, set, rest)) = tlv(sets) {
        sets = rest;
        let mut atvs = set;
        while let Some((_, atv, rest)) = tlv(atvs) {
            atvs = rest;
            let Some((0x06, oid, v)) = tlv(atv) else { continue };
            let Some((tag, value, _)) = tlv(v) else { continue };
            let key = match oid {
                [0x55, 0x04, 0x03] => "CN",
                [0x55, 0x04, 0x0a] => "O",
                [0x55, 0x04, 0x0b] => "OU",
                [0x55, 0x04, 0x06] => "C",
                [0x55, 0x04, 0x07] => "L",
                [0x55, 0x04, 0x08] => "ST",
                [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x01] => "E",
                _ => continue,
            };
            out.push((key, string(tag, value)));
        }
    }
    out
}

/// What Windows shows for a name (its "simple display name"): the common name, else the
/// organizational unit, the organization or the e-mail address.
fn display_name(name: &[u8]) -> String {
    let attrs = attributes(name);
    for key in ["CN", "OU", "O", "E"] {
        if let Some((_, v)) = attrs.iter().find(|(k, v)| *k == key && !v.is_empty()) {
            return v.clone();
        }
    }
    attrs.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ")
}

fn string(tag: u8, b: &[u8]) -> String {
    match tag {
        // BMPString: UTF-16BE
        0x1e => String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>()),
        // UniversalString: UTF-32BE
        0x1c => b.chunks_exact(4).filter_map(|c| char::from_u32(u32::from_be_bytes([c[0], c[1], c[2], c[3]]))).collect(),
        // UTF8String, PrintableString, IA5String, TeletexString (Latin-1 in practice)…
        0x14 => b.iter().map(|&c| c as char).collect(),
        _ => String::from_utf8_lossy(b).into_owned(),
    }
}

/// UTCTime (`YYMMDDHHMMSSZ`) or GeneralizedTime (`YYYYMMDDHHMMSSZ`) as Unix milliseconds.
fn time(tag: u8, b: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(b).ok()?;
    let digits = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (year, rest) = match tag {
        0x17 => {
            let yy = digits(0..2)?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, 2)
        }
        0x18 => (digits(0..4)?, 4),
        _ => return None,
    };
    let (mo, d, h, mi) = (digits(rest..rest + 2)?, digits(rest + 2..rest + 4)?, digits(rest + 4..rest + 6)?, digits(rest + 6..rest + 8)?);
    let sec = digits(rest + 8..rest + 10).unwrap_or(0);
    Some((days_from_civil(year, mo, d) * 86_400 + h * 3600 + mi * 60 + sec) * 1000)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(time(0x17, b"491231235959Z"), Some(2_524_607_999_000));
        assert_eq!(time(0x18, b"20380119031407Z"), Some(2_147_483_647_000));
    }

    #[test]
    fn base64_decodes() {
        assert_eq!(base64("aGVs\nbG8=").as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn system_certificates_parse() {
        // every distribution ships its CA bundle as PEM files here
        let Ok(dir) = std::fs::read_dir("/etc/ssl/certs") else { return };
        let mut n = 0;
        for e in dir.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "pem")).take(40) {
            let i = info(&e.path()).unwrap_or_else(|| panic!("{}", e.path().display()));
            assert!(!i.subject.is_empty() && !i.issuer.is_empty() && i.not_before < i.not_after, "{i:?}");
            n += 1;
        }
        if n > 0 {
            eprintln!("{n} certificates read");
        }
    }
}
