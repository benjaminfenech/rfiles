//! Shell icons and thumbnails via IShellItemImageFactory, decoded to BGRA pixels.
//! Blocking + COM: call from the worker pool (`ThumbPool`), never the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
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
/// Thumbnails of images the `image` crate decodes, scaled to fit `size`. File-type icons
/// (freedesktop icon theme) and the shared `~/.cache/thumbnails` cache are Phase 1: until then
/// anything else is an error and the UI shows its built-in icon.
pub fn get_image(path: &Path, size: u32, kind: Kind) -> anyhow::Result<Bgra> {
    anyhow::ensure!(kind == Kind::Thumbnail, "file-type icons are not implemented on Linux yet");
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    anyhow::ensure!(image::ImageFormat::from_extension(&ext).is_some(), "no thumbnailer for .{ext}");
    let img = image::ImageReader::open(path)?.with_guessed_format()?.decode()?;
    let rgba = img.thumbnail(size, size).into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut data = rgba.into_raw();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2); // RGBA -> BGRA
    }
    Ok(Bgra { width, height, data })
}
/// Extensions whose icon differs per file (must not be cached by extension).
pub fn has_unique_icon(ext: &str) -> bool {
    matches!(
        ext, "exe" | "lnk" | "ico" | "url" | "cur" | "ani" | "scr" | "cpl" | "msc" |
        "appref-ms" | "website"
    )
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
