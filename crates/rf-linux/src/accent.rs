//! The user's Windows accent color palette (Settings → Personalization → Colors), so the app
//! wears the same color as the taskbar, Start and Files.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
/// Shades of the accent, as 0xRRGGBB: `[light3, light2, light1, base, dark1, dark2, dark3]`.
/// Fluent uses `light2` for accents on dark surfaces and `dark1` on light ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccentPalette {
    pub shades: [u32; 7],
}
impl AccentPalette {
    pub fn light2(&self) -> u32 {
        self.shades[1]
    }
    pub fn dark1(&self) -> u32 {
        self.shades[4]
    }
    pub fn base(&self) -> u32 {
        self.shades[3]
    }
}
/// `AccentPalette` is 8 RGBA quadruples; the first seven are the shades.
pub fn parse_palette(raw: &[u8]) -> Option<AccentPalette> {
    if raw.len() < 28 {
        return None;
    }
    let mut shades = [0u32; 7];
    for (i, s) in shades.iter_mut().enumerate() {
        let c = &raw[i * 4..i * 4 + 4];
        *s = (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
    }
    Some(AccentPalette { shades })
}
/// Reads the palette from HKCU; `None` if unavailable (then use Fluent's default blue).
pub fn palette() -> Option<AccentPalette> {
    None
}
