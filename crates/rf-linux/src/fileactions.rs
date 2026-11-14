//! Type-specific file actions from Files' context menu: set an image as the desktop background,
//! install a font for the current user. Blocking: off the UI thread.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, bail};

pub fn is_image(p: &Path) -> bool {
    let e = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(e.as_str(), "jpg" | "jpeg" | "png" | "bmp" | "gif" | "tif" | "tiff" | "webp" | "jfif" | "heic" | "avif")
}

pub fn is_font(p: &Path) -> bool {
    let e = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(e.as_str(), "ttf" | "otf" | "ttc" | "fon")
}

/// Whether the desktop can set a wallpaper for apps (the portal's Wallpaper interface: KDE
/// Plasma and GNOME have it, wlroots and Hyprland portals don't). Checked once, in the
/// background: `false` until the answer is in.
pub fn can_set_wallpaper() -> bool {
    static ANSWER: OnceLock<bool> = OnceLock::new();
    static STARTED: std::sync::Once = std::sync::Once::new();
    if let Some(a) = ANSWER.get() {
        return *a;
    }
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            let ok = futures::executor::block_on(async {
                ashpd::desktop::wallpaper::WallpaperProxy::new().await.is_ok_and(|p| p.version() > 0)
            });
            let _ = ANSWER.set(ok);
        });
    });
    false
}

/// Asks the desktop to use `path` as the wallpaper (it may show a preview to confirm).
pub fn set_wallpaper(path: &Path) -> anyhow::Result<()> {
    use ashpd::desktop::wallpaper::{SetOn, WallpaperRequest};
    let file = std::fs::File::open(path).with_context(|| path.display().to_string())?;
    futures::executor::block_on(async {
        WallpaperRequest::default().show_preview(true).set_on(SetOn::Both).build_file(&file).await?.response()
    })
    .map_err(|e| anyhow::anyhow!("the desktop couldn't set the wallpaper ({e})"))
}

/// Installs a font for the current user: copies it into `~/.local/share/fonts` (where
/// fontconfig looks) and refreshes the font cache. Returns the installed copy.
pub fn install_font_for_user(path: &Path) -> anyhow::Result<PathBuf> {
    let Some(name) = path.file_name() else { bail!("not a file: {}", path.display()) };
    let dir = dirs::data_dir().context("no data folder")?.join("fonts");
    std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    let dest = dir.join(name);
    if dest.exists() {
        bail!("{} is already installed", name.to_string_lossy());
    }
    std::fs::copy(path, &dest).with_context(|| dest.display().to_string())?;
    // fontconfig notices new files on its own after a while; fc-cache makes it immediate
    let mut fc = crate::sandbox::tool("fc-cache", &[]);
    fc.arg(&dir);
    let _ = crate::tool::run(fc, std::time::Duration::from_secs(60));
    Ok(dest)
}
