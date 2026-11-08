//! Reads what DirectLook needs from a HEIF container (HEIC, AVIF) without decoding it: the
//! primary image's size with its rotation applied, and the Exif block. Only the `meta` box is
//! walked (ISO/IEC 23008-12); pixels are decoded by external tools (see `decode.rs`).

/// One box: its type and contents (header removed).
struct Bx<'a> {
    kind: [u8; 4],
    body: &'a [u8],
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The boxes laid one after another in `data`; stops at the first malformed one. A box cut
/// short by the end of `data` is returned shortened.
fn boxes(mut data: &[u8]) -> impl Iterator<Item = Bx<'_>> {
    std::iter::from_fn(move || {
        let size = be32(data, 0)? as u64;
        let kind: [u8; 4] = data.get(4..8)?.try_into().ok()?;
        let (head, size) = match size {
            0 => (8, data.len() as u64),
            1 => (16, u64::from_be_bytes(data.get(8..16)?.try_into().ok()?)),
            n => (8, n),
        };
        if size < head as u64 {
            data = &[];
            return None;
        }
        let end = (size.min(data.len() as u64)) as usize;
        let body = &data[head.min(end)..end];
        data = &data[end..];
        Some(Bx { kind, body })
    })
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).find(|b| &b.kind == kind).map(|b| b.body)
}

/// The primary image's size as shown (`irot` applied), from the header alone.
pub fn primary_size(data: &[u8]) -> Option<(u32, u32)> {
    // `meta` is a full box: 4 bytes of version and flags before its children
    let meta = child(data, b"meta")?.get(4..)?;
    let pitm = child(meta, b"pitm")?;
    let primary = if pitm.first()? == &0 {
        be16(pitm, 4)?
    } else {
        be32(pitm, 4)?
    };
    let iprp = child(meta, b"iprp")?;
    let props: Vec<Bx> = boxes(child(iprp, b"ipco")?).collect();
    let ipma = child(iprp, b"ipma")?;
    let (version, flags) = (*ipma.first()?, *ipma.get(3)?);
    let mut at = 8;
    let mut size = None;
    let mut quarter_turns = 0;
    for _ in 0..be32(ipma, 4)? {
        let item = if version < 1 {
            at += 2;
            be16(ipma, at - 2)?
        } else {
            at += 4;
            be32(ipma, at - 4)?
        };
        let count = *ipma.get(at)? as usize;
        at += 1;
        for _ in 0..count {
            // property indexes count from 1; the top bit marks "essential"
            let index = if flags & 1 != 0 {
                at += 2;
                be16(ipma, at - 2)? & 0x7FFF
            } else {
                at += 1;
                (ipma[at - 1] & 0x7F) as u32
            } as usize;
            if item != primary || index == 0 {
                continue;
            }
            let Some(p) = props.get(index - 1) else {
                continue;
            };
            match &p.kind {
                // full box: version and flags, then width and height
                b"ispe" => size = Some((be32(p.body, 4)?, be32(p.body, 8)?)),
                // anticlockwise quarter turns in the low two bits
                b"irot" => quarter_turns = p.body.first()? & 3,
                _ => {}
            }
        }
    }
    let (w, h) = size?;
    (w > 0 && h > 0).then_some(if quarter_turns % 2 == 1 {
        (h, w)
    } else {
        (w, h)
    })
}

/// Reads a big-endian number `n` bytes wide (0, 4 or 8) at `*at` and moves past it.
fn sized(b: &[u8], at: &mut usize, n: u8) -> Option<u64> {
    let v = match n {
        0 => 0,
        4 => be32(b, *at)? as u64,
        8 => u64::from_be_bytes(b.get(*at..*at + 8)?.try_into().ok()?),
        _ => return None,
    };
    *at += n as usize;
    Some(v)
}

/// The TIFF data of the Exif item, found through `iinf` (which item is Exif) and `iloc` (where
/// its bytes are). `data` must be the whole file: the item usually sits in `mdat`.
pub fn exif(data: &[u8]) -> Option<&[u8]> {
    let meta = child(data, b"meta")?.get(4..)?;
    let iinf = child(meta, b"iinf")?;
    let count_len = if *iinf.first()? == 0 { 2 } else { 4 };
    let id = boxes(iinf.get(4 + count_len..)?)
        .filter(|b| &b.kind == b"infe")
        .find_map(|b| {
            // version 2: 16-bit item id; version 3: 32-bit; then a 16-bit protection index, the type
            let (id, kind_at) = match b.body.first()? {
                2 => (be16(b.body, 4)?, 8),
                3 => (be32(b.body, 4)?, 10),
                _ => return None,
            };
            (b.body.get(kind_at..kind_at + 4)? == b"Exif").then_some(id)
        })?;
    let iloc = child(meta, b"iloc")?;
    let version = *iloc.first()?;
    let (off_size, len_size) = (iloc.get(4)? >> 4, iloc.get(4)? & 15);
    let base_size = iloc.get(5)? >> 4;
    let index_size = if version == 1 || version == 2 {
        iloc.get(5)? & 15
    } else {
        0
    };
    // `sized` reads 0, 4 or 8 bytes; counts and ids may be 2 bytes wide
    let two = |at: &mut usize| {
        *at += 2;
        be16(iloc, *at - 2)
    };
    let mut at = 6;
    let count = if version < 2 {
        two(&mut at)?
    } else {
        sized(iloc, &mut at, 4)? as u32
    };
    for _ in 0..count {
        let item = if version < 2 {
            two(&mut at)?
        } else {
            sized(iloc, &mut at, 4)? as u32
        };
        let method = if version == 1 || version == 2 {
            two(&mut at)? & 15
        } else {
            0
        };
        at += 2; // data reference index
        let base = sized(iloc, &mut at, base_size)?;
        let mut first = None;
        for e in 0..two(&mut at)? {
            sized(iloc, &mut at, index_size)?;
            let off = sized(iloc, &mut at, off_size)?;
            let len = sized(iloc, &mut at, len_size)?;
            if e == 0 {
                first = Some((off, len));
            }
        }
        if item != id {
            continue;
        }
        let (off, len) = first?;
        // method 0: offsets into the file; 1: into the `idat` box
        let src = match method {
            0 => data,
            1 => child(meta, b"idat")?,
            _ => return None,
        };
        let start = usize::try_from(base + off).ok()?;
        let end = if len == 0 {
            src.len()
        } else {
            start.saturating_add(len as usize).min(src.len())
        };
        let payload = src.get(start..end)?;
        // a 32-bit offset to the TIFF header (past an optional "Exif\0\0"), then TIFF
        let tiff = payload.get(4 + be32(payload, 0)? as usize..)?;
        return (tiff.starts_with(b"II*\0") || tiff.starts_with(b"MM\0*")).then_some(tiff);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    /// A HEIF header: item 1 (a thumbnail, 64 x 48) and the primary item 2 (4000 x 3000,
    /// optionally turned a quarter).
    fn heif(irot: Option<u8>) -> Vec<u8> {
        let mut ipco = bx(b"ispe", &[0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 48]);
        ipco.extend(bx(
            b"ispe",
            &[0, 0, 0, 0, 0, 0, 0x0F, 0xA0, 0, 0, 0x0B, 0xB8],
        ));
        let mut ipma = vec![0, 0, 0, 0, 0, 0, 0, 2];
        ipma.extend([0, 1, 1, 0x81]); // item 1: property 1 (essential)
        if let Some(r) = irot {
            ipco.extend(bx(b"irot", &[r]));
            ipma.extend([0, 2, 2, 0x82, 0x03]); // item 2: properties 2 and 3
        } else {
            ipma.extend([0, 2, 1, 0x82]);
        }
        let mut iprp = bx(b"ipco", &ipco);
        iprp.extend(bx(b"ipma", &ipma));
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(bx(b"hdlr", &[0; 24]));
        meta.extend(bx(b"pitm", &[0, 0, 0, 0, 0, 2]));
        meta.extend(bx(b"iprp", &iprp));
        let mut file = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        file.extend(bx(b"meta", &meta));
        file
    }

    #[test]
    fn reads_the_primary_size() {
        assert_eq!(primary_size(&heif(None)), Some((4000, 3000)));
        assert_eq!(primary_size(&heif(Some(1))), Some((3000, 4000)));
        assert_eq!(primary_size(&heif(Some(2))), Some((4000, 3000)));
        let full = heif(None);
        assert_eq!(primary_size(&full[..full.len() / 2]), None);
        assert_eq!(primary_size(b"not a heif file"), None);
    }

    #[test]
    fn finds_the_exif_item() {
        // avifenc from a JPEG whose EXIF says Make = "TestMake"
        let data =
            std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/exif.avif")).unwrap();
        let tiff = exif(&data).expect("exif item");
        assert!(tiff.starts_with(b"MM\0*") || tiff.starts_with(b"II*\0"));
        assert!(tiff.windows(8).any(|w| w == b"TestMake"));
        assert_eq!(exif(&heif(None)), None);
    }
}
