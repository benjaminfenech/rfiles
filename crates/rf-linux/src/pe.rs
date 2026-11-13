//! Programs and libraries (EXE, DLL, SYS…): architecture from the PE header, plus the version
//! resource's file version, product name, company and description. Read-only.
//!
//! Linux: the header part is the same code as rf-win; rf-win reads the version resource with a
//! Windows API, here it is walked by hand (resource directory → RT_VERSION → VS_VERSIONINFO).

use std::io::Read;
use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct PeInfo {
    /// "x64", "x86", "ARM64"…
    pub arch: &'static str,
    pub version: Option<String>,
    pub product: Option<String>,
    pub company: Option<String>,
    pub description: Option<String>,
    pub product_version: Option<String>,
    pub copyright: Option<String>,
    pub original_filename: Option<String>,
    pub language: Option<String>,
    pub trademarks: Option<String>,
    pub internal_name: Option<String>,
}

fn machine_name(m: u16) -> &'static str {
    match m {
        0x8664 => "x64",
        0x014c => "x86",
        0xaa64 => "ARM64",
        0x01c4 => "ARM",
        0xa641 => "ARM64EC",
        0x0200 => "IA-64",
        _ => "?",
    }
}

/// The PE machine field, or None when the file isn't a PE image.
fn machine(head: &[u8]) -> Option<u16> {
    if head.len() < 0x40 || &head[..2] != b"MZ" {
        return None;
    }
    let off = u32::from_le_bytes(head[0x3c..0x40].try_into().ok()?) as usize;
    let sig = head.get(off..off + 6)?;
    if &sig[..4] != b"PE\0\0" {
        return None;
    }
    Some(u16::from_le_bytes([sig[4], sig[5]]))
}

pub fn info(path: &Path) -> Option<PeInfo> {
    let mut head = vec![0u8; 4096];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let arch = machine_name(machine(&head[..n])?);
    let mut out = PeInfo { arch, ..Default::default() };
    // the version resource can sit anywhere in the file: read it whole (programs over 256 MB
    // keep just their architecture)
    if std::fs::metadata(path).is_ok_and(|m| m.len() <= 256 << 20)
        && let Ok(data) = std::fs::read(path)
    {
        version_strings(&data, &mut out);
    }
    Some(out)
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// The file offset of a relative virtual address, through the section table.
fn rva_to_offset(data: &[u8], sections: usize, count: usize, rva: u32) -> Option<usize> {
    for i in 0..count {
        let s = sections + i * 40;
        let (vsize, vaddr, raw_size, raw_ptr) = (u32_at(data, s + 8)?, u32_at(data, s + 12)?, u32_at(data, s + 16)?, u32_at(data, s + 20)?);
        if rva >= vaddr && rva < vaddr + vsize.max(raw_size) {
            return Some((rva - vaddr + raw_ptr) as usize);
        }
    }
    None
}

/// The `RT_VERSION` resource's bytes: resource directory → type 16 → its first name → its
/// first language → the data.
fn version_resource(data: &[u8]) -> Option<&[u8]> {
    let pe = u32_at(data, 0x3c)? as usize;
    let count = u16_at(data, pe + 6)? as usize;
    let opt_size = u16_at(data, pe + 20)? as usize;
    let opt = pe + 24;
    // the data directories start at 96 (PE32) or 112 (PE32+); resources are the third
    let dirs = opt + if u16_at(data, opt)? == 0x20b { 112 } else { 96 };
    let rsrc_rva = u32_at(data, dirs + 2 * 8)?;
    if rsrc_rva == 0 {
        return None;
    }
    let sections = opt + opt_size;
    let base = rva_to_offset(data, sections, count, rsrc_rva)?;
    // entries of the directory at `dir`: (id or name, offset with its high bit = subdirectory)
    let entries = |dir: usize| -> Option<Vec<(u32, u32)>> {
        let n = u16_at(data, dir + 12)? as usize + u16_at(data, dir + 14)? as usize;
        (0..n.min(4096)).map(|i| Some((u32_at(data, dir + 16 + i * 8)?, u32_at(data, dir + 20 + i * 8)?))).collect()
    };
    let sub = |off: u32| (off & 0x8000_0000 != 0).then(|| base + (off & 0x7fff_ffff) as usize);
    let (_, types) = entries(base)?.into_iter().find(|(id, _)| *id == 16)?;
    let names = sub(types)?;
    let (_, langs) = *entries(names)?.first()?;
    let langs = sub(langs)?;
    // one copy per language in localized programs (Wine's): the same choice as the tables'
    let (_, leaf) = entries(langs)?.into_iter().min_by_key(|(id, _)| language_rank(Some(*id as u16)))?;
    if leaf & 0x8000_0000 != 0 {
        return None;
    }
    let leaf = base + leaf as usize;
    let (rva, size) = (u32_at(data, leaf)?, u32_at(data, leaf + 4)? as usize);
    let off = rva_to_offset(data, sections, count, rva)?;
    data.get(off..off + size)
}

/// A version block's header: (length, value length, is text, key, where the value starts).
fn block(b: &[u8], at: usize) -> Option<(usize, usize, bool, String, usize)> {
    let len = u16_at(b, at)? as usize;
    let vlen = u16_at(b, at + 2)? as usize;
    let text = u16_at(b, at + 4)? == 1;
    let mut key = Vec::new();
    let mut p = at + 6;
    loop {
        let c = u16_at(b, p)?;
        p += 2;
        if c == 0 {
            break;
        }
        key.push(c);
    }
    let value = (p + 3) & !3;
    Some((len, vlen, text, String::from_utf16_lossy(&key), value))
}

/// The children of the block at `at` (each starts on a 4-byte boundary).
fn children(b: &[u8], at: usize, len: usize, first: usize) -> Vec<usize> {
    let end = (at + len).min(b.len());
    let mut out = Vec::new();
    let mut p = (first + 3) & !3;
    while p + 6 <= end {
        let Some(l) = u16_at(b, p).map(|l| l as usize) else { break };
        if l == 0 {
            break;
        }
        out.push(p);
        p = (p + l + 3) & !3;
    }
    out
}

fn utf16z(b: &[u8], at: usize, chars: usize) -> String {
    let units: Vec<u16> = (0..chars).map_while(|i| u16_at(b, at + i * 2)).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&units).trim().to_string()
}

/// VS_VERSIONINFO: the fixed file version, then the first string table's strings.
fn version_strings(data: &[u8], out: &mut PeInfo) {
    let Some(v) = version_resource(data) else { return };
    let Some((len, vlen, _, key, value)) = block(v, 0) else { return };
    if key != "VS_VERSION_INFO" {
        return;
    }
    // VS_FIXEDFILEINFO: signature, struct version, file version (MS, LS)
    if vlen >= 52 && u32_at(v, value) == Some(0xFEEF_04BD) {
        let (ms, ls) = (u32_at(v, value + 8).unwrap_or(0), u32_at(v, value + 12).unwrap_or(0));
        out.version = Some(format!("{}.{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16, ls & 0xffff));
    }
    for child in children(v, 0, len, value + vlen) {
        let Some((clen, _, _, ckey, cvalue)) = block(v, child) else { continue };
        if ckey != "StringFileInfo" {
            continue;
        }
        // one string table per language: US English first (what most programs ship), then
        // neutral or another English, then whichever comes first
        let tables: Vec<(usize, usize, usize, Option<u16>)> = children(v, child, clen, cvalue)
            .into_iter()
            .filter_map(|t| block(v, t).map(|(tlen, _, _, lang, tvalue)| (t, tlen, tvalue, u16::from_str_radix(lang.get(..4).unwrap_or(""), 16).ok())))
            .collect();
        let Some(&(table, tlen, tvalue, lang)) = tables.iter().min_by_key(|t| language_rank(t.3)) else { continue };
        out.language = lang.map(language_name);
        for s in children(v, table, tlen, tvalue) {
            let Some((_, svlen, _, skey, svalue)) = block(v, s) else { continue };
            let text = utf16z(v, svalue, svlen);
            if text.is_empty() {
                continue;
            }
            let slot = match skey.as_str() {
                "FileDescription" => &mut out.description,
                "ProductName" => &mut out.product,
                "CompanyName" => &mut out.company,
                "ProductVersion" => &mut out.product_version,
                "LegalCopyright" => &mut out.copyright,
                "OriginalFilename" => &mut out.original_filename,
                "LegalTrademarks" => &mut out.trademarks,
                "InternalName" => &mut out.internal_name,
                // the fixed version wins: it's what Windows shows
                "FileVersion" if out.version.is_none() => &mut out.version,
                _ => continue,
            };
            *slot = Some(text);
        }
    }
}

/// Which language's strings to show: US English (what most programs ship), then neutral or
/// another English, then any.
fn language_rank(l: Option<u16>) -> u8 {
    match l {
        Some(0x0409) => 0,
        Some(0x0000 | 0x007f) => 1,
        Some(l) if l & 0x3ff == 0x09 => 2,
        _ => 3,
    }
}

/// A Windows language id's name ("English (United States)"), for the common ones.
fn language_name(id: u16) -> String {
    match id {
        0x0000 | 0x007f => "Language neutral",
        0x0403 => "Catalan",
        0x0409 => "English (United States)",
        0x0809 => "English (United Kingdom)",
        0x041f => "Turkish",
        0x0407 => "German",
        0x040c => "French",
        0x0410 => "Italian",
        0x0c0a | 0x040a => "Spanish",
        0x0416 => "Portuguese (Brazil)",
        0x0816 => "Portuguese (Portugal)",
        0x0413 => "Dutch",
        0x0419 => "Russian",
        0x0411 => "Japanese",
        0x0412 => "Korean",
        0x0804 => "Chinese (Simplified)",
        0x0404 => "Chinese (Traditional)",
        0x0415 => "Polish",
        0x041d => "Swedish",
        0x0405 => "Czech",
        _ => return format!("0x{id:04x}"),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn non_pe_is_none() {
        assert_eq!(super::machine(b"hello"), None);
        let mut h = vec![0u8; 0x100];
        h[..2].copy_from_slice(b"MZ");
        h[0x3c] = 0x80;
        h[0x80..0x86].copy_from_slice(b"PE\0\0\x64\x86");
        assert_eq!(super::machine(&h), Some(0x8664));
    }

    #[test]
    fn version_resource_is_read() {
        // Wine's DLLs carry a version resource; skipped where Wine isn't installed
        for p in ["/usr/lib/wine/x86_64-windows/kernel32.dll", "/usr/lib/x86_64-linux-gnu/wine/x86_64-windows/kernel32.dll", "/usr/lib64/wine/x86_64-windows/kernel32.dll"] {
            let Some(i) = super::info(std::path::Path::new(p)) else { continue };
            assert_eq!(i.arch, "x64");
            assert!(i.version.as_deref().is_some_and(|v| v.split('.').count() == 4), "{i:?}");
            assert!(i.description.as_deref().is_some_and(|d| d.contains("kernel")), "{i:?}");
            assert_eq!(i.language.as_deref(), Some("English (United States)"));
            assert_eq!(i.original_filename.as_deref(), Some("kernel32.dll"));
            return;
        }
    }

    #[test]
    fn language_names() {
        assert_eq!(super::language_name(0x0409), "English (United States)");
        assert_eq!(super::language_name(0x1234), "0x1234");
    }
}
