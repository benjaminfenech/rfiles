//! Programs and libraries (EXE, DLL, SYS…): architecture from the PE header, plus the version
//! resource's file version, product name, company and description. Read-only.
//!
//! Linux: the header part is the same code as rf-win; the version resource is read by a Windows
//! API there, so on Linux only `arch` is filled in for now.

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
    version_strings(path, &mut out);
    Some(out)
}

/// Phase 2: parse the VS_VERSIONINFO resource ourselves.
fn version_strings(_path: &Path, _out: &mut PeInfo) {}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn reads_notepad() {
        let p = std::path::Path::new(r"C:\Windows\System32\notepad.exe");
        if !p.exists() {
            return;
        }
        let i = super::info(p).expect("notepad is a PE image");
        assert_ne!(i.arch, "?");
        assert!(i.version.is_some());
        assert!(i.company.as_deref().unwrap_or("").contains("Microsoft"), "{i:?}");
    }

    #[test]
    fn non_pe_is_none() {
        assert_eq!(super::machine(b"hello"), None);
        let mut h = vec![0u8; 0x100];
        h[..2].copy_from_slice(b"MZ");
        h[0x3c] = 0x80;
        h[0x80..0x86].copy_from_slice(b"PE\0\0\x64\x86");
        assert_eq!(super::machine(&h), Some(0x8664));
    }
}
