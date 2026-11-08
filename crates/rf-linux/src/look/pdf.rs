//! PDF pages through poppler's command-line tools: `pdfinfo` reads the page count and sizes
//! once, and `pdftoppm` renders one page at a time to PPM on its stdout. No library to link;
//! poppler is installed almost everywhere (it also drives most PDF thumbnailers).
//!
//! Both tools run as child processes and block: use a worker thread, never the UI thread.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};

use crate::thumbs::Bgra;

/// Widest page we render (physical pixels); wider requests are capped.
pub const MAX_WIDTH: u32 = 4096;

/// How long `pdfinfo` may take to read the document.
const INFO_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `pdftoppm` may take for one page.
const RENDER_TIMEOUT: Duration = Duration::from_secs(30);

/// An open PDF.
pub struct Pdf {
    path: PathBuf,
    password: Option<String>,
    /// Page sizes in DIPs, rotation applied.
    sizes: Vec<(f32, f32)>,
}

/// Why a PDF can't be shown.
#[derive(Debug, PartialEq, Eq)]
pub enum PdfError {
    /// Protected with a password.
    Password,
    /// Not readable as a PDF (damaged, or not a PDF).
    Unreadable,
    /// poppler's tools (`pdfinfo`, `pdftoppm`) are not installed.
    NoPoppler,
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PdfError::Password => "the PDF is protected with a password",
            PdfError::Unreadable => "the file can't be read as a PDF",
            PdfError::NoPoppler => "poppler (pdfinfo, pdftoppm) is not installed",
        })
    }
}

impl std::error::Error for PdfError {}

/// A path the tools won't mistake for an option.
fn path_arg(path: &Path) -> PathBuf {
    if path.as_os_str().as_encoded_bytes().starts_with(b"-") {
        Path::new(".").join(path)
    } else {
        path.to_path_buf()
    }
}

struct Ran {
    ok: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Runs `cmd`, collecting its output; kills it after `timeout`.
fn run(mut cmd: Command, timeout: Duration) -> anyhow::Result<Ran> {
    let prog = cmd.get_program().to_string_lossy().into_owned();
    let mut child = match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PdfError::NoPoppler.into());
        }
        Err(e) => return Err(anyhow!("{prog}: {e}")),
    };
    // read both pipes on their own threads so a full pipe never stalls the child
    let reader = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = reader(child.stdout.take().map(|s| Box::new(s) as _));
    let err = reader(child.stderr.take().map(|s| Box::new(s) as _));
    let started = Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(s) => break s,
            None if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("{prog} took longer than {timeout:?}");
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    Ok(Ran {
        ok: status.success(),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

fn with_password(cmd: &mut Command, password: Option<&str>) {
    if let Some(p) = password {
        cmd.arg("-opw").arg(p).arg("-upw").arg(p);
    }
}

/// Page sizes in points from `pdfinfo -box` output, rotation applied. Uses the crop box (what
/// `pdftoppm` renders), else the plain size line.
fn parse_info(text: &str) -> Vec<(f32, f32)> {
    #[derive(Clone, Default)]
    struct PageInfo {
        size: Option<(f32, f32)>,
        crop: Option<(f32, f32)>,
        rot: u32,
    }
    let mut info: Vec<PageInfo> = Vec::new();
    let nums = |s: &str| {
        s.split_whitespace()
            .filter_map(|t| t.parse::<f32>().ok())
            .collect::<Vec<_>>()
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Pages:") {
            let pages: u32 = rest.trim().parse().unwrap_or(0);
            info = vec![PageInfo::default(); pages.min(100_000) as usize];
            continue;
        }
        let Some(rest) = line.strip_prefix("Page ") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some((n, rest)) = rest.split_once(' ') else {
            continue;
        };
        let Some(slot) = n
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| info.get_mut(i))
        else {
            continue;
        };
        let rest = rest.trim_start();
        if let Some(v) = rest.strip_prefix("size:") {
            // "595.28 x 841.89 pts (A4)"
            let v: Vec<f32> = nums(&v.replace('x', " "));
            if v.len() >= 2 {
                slot.size = Some((v[0], v[1]));
            }
        } else if let Some(v) = rest.strip_prefix("CropBox:") {
            let v = nums(v);
            if v.len() == 4 {
                slot.crop = Some(((v[2] - v[0]).abs(), (v[3] - v[1]).abs()));
            }
        } else if let Some(v) = rest.strip_prefix("rot:") {
            slot.rot = v.trim().parse::<i64>().unwrap_or(0).rem_euclid(360) as u32;
        }
    }
    info.into_iter()
        .map(|p| {
            let (w, h) = p.crop.or(p.size).unwrap_or((595.0, 842.0));
            if p.rot == 90 || p.rot == 270 {
                (h, w)
            } else {
                (w, h)
            }
        })
        .collect()
}

/// A binary PPM (P6, 8 bit) from `pdftoppm`, as BGRA.
fn parse_ppm(data: &[u8]) -> anyhow::Result<Bgra> {
    // header: "P6" w h maxval, separated by whitespace, then one whitespace byte
    let mut fields = Vec::with_capacity(4);
    let mut i = 0;
    while fields.len() < 4 {
        while data.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
            i += 1;
        }
        if data.get(i) == Some(&b'#') {
            while data.get(i).is_some_and(|&b| b != b'\n') {
                i += 1;
            }
            continue;
        }
        let start = i;
        while data.get(i).is_some_and(|b| !b.is_ascii_whitespace()) {
            i += 1;
        }
        anyhow::ensure!(i > start, "truncated PPM header");
        fields.push(std::str::from_utf8(&data[start..i])?);
    }
    i += 1;
    anyhow::ensure!(
        fields[0] == "P6" && fields[3] == "255",
        "unexpected PPM format {} / {}",
        fields[0],
        fields[3]
    );
    let w: u32 = fields[1].parse()?;
    let h: u32 = fields[2].parse()?;
    let px = data.get(i..).unwrap_or_default();
    let n = (w as usize) * (h as usize);
    anyhow::ensure!(
        px.len() >= n * 3,
        "truncated PPM: {} of {} bytes",
        px.len(),
        n * 3
    );
    let mut out = Vec::with_capacity(n * 4);
    for rgb in px[..n * 3].as_chunks::<3>().0 {
        out.extend_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
    }
    Ok(Bgra {
        width: w,
        height: h,
        data: out,
    })
}

impl Pdf {
    pub fn open(path: &Path, password: Option<&str>) -> anyhow::Result<Pdf> {
        let mut cmd = Command::new("pdfinfo");
        cmd.arg("-box").args(["-f", "1", "-l", "2147483647"]);
        with_password(&mut cmd, password);
        cmd.arg(path_arg(path));
        let ran = run(cmd, INFO_TIMEOUT)?;
        if !ran.ok {
            let err = String::from_utf8_lossy(&ran.stderr).to_lowercase();
            if err.contains("password") {
                return Err(PdfError::Password.into());
            }
            log::debug!("pdfinfo {}: {}", path.display(), err.trim());
            return Err(PdfError::Unreadable.into());
        }
        let sizes = parse_info(&String::from_utf8_lossy(&ran.stdout))
            .into_iter()
            .map(|(w, h)| (w * 96.0 / 72.0, h * 96.0 / 72.0))
            .collect();
        Ok(Pdf {
            path: path.to_path_buf(),
            password: password.map(str::to_owned),
            sizes,
        })
    }

    pub fn page_count(&self) -> u32 {
        self.sizes.len() as u32
    }

    /// Page size in DIPs (1/96 inch), as the page is shown (rotation applied).
    pub fn page_size(&self, index: u32) -> anyhow::Result<(f32, f32)> {
        self.sizes
            .get(index as usize)
            .copied()
            .ok_or_else(|| anyhow!("no page {index}"))
    }

    /// Renders page `index` (0-based) `width` pixels wide, on white.
    pub fn render(&self, index: u32, width: u32) -> anyhow::Result<Bgra> {
        let (pw, ph) = self.page_size(index)?;
        let width = width.clamp(16, MAX_WIDTH);
        // `-scale-to-x/-y` apply to the page before its rotation; `-scale-to` (the longer side)
        // is right either way
        let height = ((width as f32) * ph / pw.max(1.0)).round().max(1.0) as u32;
        let longer = width.max(height).min(MAX_WIDTH * 4);
        let page = (index + 1).to_string();
        let mut cmd = Command::new("pdftoppm");
        cmd.args(["-f", &page, "-l", &page, "-singlefile", "-scale-to"])
            .arg(longer.to_string());
        with_password(&mut cmd, self.password.as_deref());
        // no output root: the PPM goes to stdout
        cmd.arg(path_arg(&self.path));
        let ran = run(cmd, RENDER_TIMEOUT)?;
        if !ran.ok {
            anyhow::bail!(
                "pdftoppm page {page}: {}",
                String::from_utf8_lossy(&ran.stderr).trim()
            );
        }
        parse_ppm(&ran.stdout).with_context(|| format!("pdftoppm page {page}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF, 200 x 100 pt, with a filled rectangle.
    fn tiny_pdf(path: &Path, extra: &str) {
        let content = b"0 0 1 rg 20 20 160 60 re f";
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] {extra} /Contents 4 0 R >>"
            ),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                std::str::from_utf8(content).unwrap()
            ),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes(),
        );
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objs.len() + 1
            )
            .as_bytes(),
        );
        std::fs::write(path, out).unwrap();
    }

    fn have_poppler() -> bool {
        Command::new("pdftoppm")
            .arg("-v")
            .stderr(Stdio::null())
            .status()
            .is_ok()
    }

    #[test]
    fn parses_info() {
        let text = "Pages:           2\n\
            Page    1 size:  595.28 x 841.89 pts (A4)\n\
            Page    1 rot:   0\n\
            Page    1 MediaBox:      0.00     0.00   595.28   841.89\n\
            Page    1 CropBox:      10.00    10.00   510.00   710.00\n\
            Page    2 size:  200 x 100 pts\n\
            Page    2 rot:   -90\n";
        assert_eq!(parse_info(text), vec![(500.0, 700.0), (100.0, 200.0)]);
        assert_eq!(parse_info("Pages: 0\n"), vec![]);
    }

    #[test]
    fn parses_ppm() {
        let img = parse_ppm(b"P6\n# c\n2 1\n255\n\x01\x02\x03\x04\x05\x06").unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.data, vec![3, 2, 1, 255, 6, 5, 4, 255]);
        assert!(parse_ppm(b"P6\n2 2\n255\n\x01").is_err());
    }

    #[test]
    fn renders_a_page() {
        if !have_poppler() {
            eprintln!("poppler not installed; skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!("rf-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // a name starting with '-' must not be read as an option
        let p = dir.join("-tiny.pdf");
        tiny_pdf(&p, "");
        let pdf = Pdf::open(&p, None).unwrap();
        assert_eq!(pdf.page_count(), 1);
        let (w, h) = pdf.page_size(0).unwrap();
        // 200 x 100 pt = 266.7 x 133.3 DIP
        assert!(
            (w - 266.7).abs() < 1.0 && (h - 133.3).abs() < 1.0,
            "{w} x {h}"
        );
        let img = pdf.render(0, 400).unwrap();
        assert_eq!((img.width, img.height), (400, 200));
        // the middle is the blue rectangle (BGRA), a corner is white
        let at = |x: u32, y: u32| &img.data[((y * img.width + x) * 4) as usize..][..3];
        assert_eq!(at(200, 100), &[255, 0, 0]);
        assert_eq!(at(2, 2), &[255, 255, 255]);

        // a page turned a quarter is shown tall
        let r = dir.join("rotated.pdf");
        tiny_pdf(&r, "/Rotate 90");
        let pdf = Pdf::open(&r, None).unwrap();
        let (w, h) = pdf.page_size(0).unwrap();
        assert!(h > w, "{w} x {h}");
        let img = pdf.render(0, 100).unwrap();
        assert_eq!((img.width, img.height), (100, 200));

        let bad = dir.join("bad.pdf");
        std::fs::write(&bad, b"%PDF-1.4 bozuk").unwrap();
        let e = Pdf::open(&bad, None).err().unwrap();
        assert_eq!(e.downcast_ref::<PdfError>(), Some(&PdfError::Unreadable));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
