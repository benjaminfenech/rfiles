//! File icons and thumbnails, decoded to BGRA pixels. Icons come from the desktop's icon
//! theme ([`crate::xdg_icons`]); thumbnails from the shared thumbnail cache, our decoders or
//! the installed thumbnailers ([`crate::xdg_thumbs`]).
//! Blocking: call from the worker pool (`ThumbPool`), never the UI thread.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
/// Straight-alpha BGRA, top-down rows.
#[derive(Clone)]
pub struct Bgra {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Icon,
    /// Real thumbnail if the shell has one; otherwise falls back to the icon.
    Thumbnail,
}
/// Starts GDI+ once and never shuts it down.
///
/// The shell starts and stops GDI+ around icon extraction. When the last user stops it,
/// `GdiplusShutdown` holds the GDI+ lock while waiting for its helper thread to exit, and
/// a thread exit needs the loader lock. Some third-party thumbnail handlers (Corel's
/// FileInfoProvider.dll) call `GdiplusStartup` from DllMain, i.e. under the loader lock,
/// and then wait for the GDI+ lock: a deadlock that also blocks every new thread of the
/// process (the folder listing never starts). With our own reference held for the whole
/// process lifetime, the shell's shutdown calls only decrement the count and never wait.
pub fn keep_gdiplus_alive() {}
/// Straight-alpha RGBA → [`Bgra`].
pub(crate) fn rgba_to_bgra(img: image::RgbaImage) -> Bgra {
    let (width, height) = img.dimensions();
    let mut data = img.into_raw();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Bgra { width, height, data }
}

/// An icon (`Kind::Icon`) or a thumbnail, at most `size` pixels on its longer side. A file
/// without a thumbnail gets its icon at that size.
pub fn get_image(path: &Path, size: u32, kind: Kind) -> anyhow::Result<Bgra> {
    if kind == Kind::Thumbnail {
        match crate::xdg_thumbs::thumbnail(path, size) {
            Ok(img) => return Ok(rgba_to_bgra(img)),
            Err(e) => log::debug!("thumbnail {}: {e:#}", path.display()),
        }
    }
    icon(path, size).ok_or_else(|| anyhow::anyhow!("no icon for {}", path.display()))
}

/// The file-type icon from the desktop's icon theme.
fn icon(path: &Path, size: u32) -> Option<Bgra> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).ok();
    if meta.as_ref().is_some_and(|m| m.is_dir()) {
        return crate::xdg_icons::render_first(&["folder".into()], size);
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "desktop" {
        if let Some(b) = desktop_icon(path, size) {
            return Some(b);
        }
    }
    let mut names = crate::xdg_icons::mime_for_ext(&ext).map(crate::xdg_icons::icon_names_for_mime).unwrap_or_default();
    if ext.is_empty() && meta.is_some_and(|m| m.permissions().mode() & 0o111 != 0) {
        names.push("application-x-executable".into());
    }
    names.extend(["application-x-generic", "text-x-generic", "unknown"].map(String::from));
    crate::xdg_icons::render_first(&names, size)
}

/// A `.desktop` launcher's own icon (`Icon=` name or path).
fn desktop_icon(path: &Path, size: u32) -> Option<Bgra> {
    let text = std::fs::read_to_string(path).ok()?;
    let icon = text.lines().map(str::trim).find_map(|l| l.strip_prefix("Icon="))?.trim();
    if icon.starts_with('/') {
        return crate::xdg_icons::render_file(Path::new(icon), size);
    }
    crate::xdg_icons::render_first(&[icon.to_string(), "application-x-executable".into()], size)
}

/// Files whose icon differs per file (must not be cached by extension): launchers.
pub fn has_unique_icon(ext: &str) -> bool {
    ext == "desktop"
}
pub struct Request {
    pub key: u64,
    pub path: PathBuf,
    pub size: u32,
    pub kind: Kind,
}
pub struct Response {
    pub key: u64,
    pub image: Option<Arc<Bgra>>,
    /// Request was discarded (backlog) — not a failure; may be retried.
    pub dropped: bool,
}
/// Background worker pool for icon/thumbnail extraction. Newest requests are served first
/// (they belong to what the user is looking at right now).
pub struct ThumbPool {
    stack: Arc<Mutex<Vec<Request>>>,
    wake: Sender<()>,
    on_done: Arc<dyn Fn(Response) + Send + Sync>,
}
impl ThumbPool {
    pub fn new(
        workers: usize,
        on_done: impl Fn(Response) + Send + Sync + 'static,
    ) -> Self {
        let stack: Arc<Mutex<Vec<Request>>> = Arc::new(Mutex::new(Vec::new()));
        let (wake, rx): (Sender<()>, Receiver<()>) = crossbeam_channel::unbounded();
        let on_done: Arc<dyn Fn(Response) + Send + Sync> = Arc::new(on_done);
        for i in 0..workers {
            let stack = stack.clone();
            let rx = rx.clone();
            let on_done = on_done.clone();
            std::thread::Builder::new()
                .name(format!("rf-thumb-{i}"))
                .spawn(move || {
                    // lowest CPU priority: never compete with the user's work
                    // SAFETY: plain syscall on the calling thread
                    unsafe {
                        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
                    }
                    while rx.recv().is_ok() {
                        loop {
                            let req = stack.lock().pop();
                            let Some(req) = req else { break };
                            let image = get_image(&req.path, req.size, req.kind).ok().map(Arc::new);
                            on_done(Response { key: req.key, image, dropped: false });
                        }
                    }
                })
                .ok();
        }
        Self { stack, wake, on_done }
    }
    pub fn request(&self, req: Request) {
        let mut s = self.stack.lock();
        let dropped: Vec<Request> = if s.len() > 512 {
            s.drain(0..256).collect()
        } else {
            Vec::new()
        };
        s.push(req);
        drop(s);
        for d in dropped {
            (self
                .on_done)(Response {
                key: d.key,
                image: None,
                dropped: true,
            });
        }
        let _ = self.wake.send(());
    }
    /// Drops queued work (e.g. on navigation).
    pub fn clear(&self) {
        self.stack.lock().clear();
    }
}
