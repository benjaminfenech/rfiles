//! PDF pages through Windows' own PDF engine (Windows.Data.Pdf, part of Windows 10 and 11):
//! no extra library. A page renders to PNG at the width asked for and is decoded to BGRA.
//!
//! WinRT calls block (`.get()`): use a worker thread, never the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::Path;
use anyhow::{Context, anyhow};
use crate::thumbs::Bgra;
/// Widest page we render (physical pixels); wider requests are capped.
pub const MAX_WIDTH: u32 = 4096;
/// An open PDF.
pub struct Pdf {
    doc: crate::Opaque,
}
/// Why a PDF can't be shown.
#[derive(Debug, PartialEq, Eq)]
pub enum PdfError {
    /// Protected with a password.
    Password,
    /// Not readable as a PDF (damaged, or not a PDF).
    Unreadable,
}
impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            match self {
                PdfError::Password => "the PDF is protected with a password",
                PdfError::Unreadable => "the file can't be read as a PDF",
            },
        )
    }
}
impl std::error::Error for PdfError {}
/// WinRT needs the calling thread in an apartment; a worker thread may have none yet.
fn ensure_apartment() {}
impl Pdf {
    pub fn open(path: &Path, password: Option<&str>) -> anyhow::Result<Pdf> {
        Err(anyhow::anyhow!("rf-linux: `open` is not implemented on Linux yet").into())
    }
    pub fn page_count(&self) -> u32 {
        0
    }
    /// Page size in DIPs (1/96 inch), as the page is shown (rotation applied).
    pub fn page_size(&self, index: u32) -> anyhow::Result<(f32, f32)> {
        anyhow::bail!("rf-linux: PDF pages are not implemented on Linux yet")
    }
    /// Renders page `index` (0-based) `width` pixels wide, on white.
    pub fn render(&self, index: u32, width: u32) -> anyhow::Result<Bgra> {
        Err(anyhow::anyhow!("rf-linux: `render` is not implemented on Linux yet").into())
    }
}
