//! Hosts Windows preview handlers (IPreviewHandler — what Explorer's preview pane uses for
//! Office documents, PDF, video/audio, fonts, …) in a borderless window owned by the app
//! window and kept over the preview pane.
//!
//! Everything runs on the shell STA thread: a slow handler (Office can take a second) never
//! blocks the UI thread, and an *owned* window (unlike a child window) does not attach the two
//! threads' input queues.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use crate::wide::wide;
const PREVIEW_HANDLER_IID: &str = "{8895b1c6-b41f-4c1c-a562-0d564250836f}";
/// CLSID of the preview handler registered for `path`'s type, if any.
pub fn handler_for(path: &Path) -> Option<crate::Opaque> {
    None
}
struct Host {
    hwnd: crate::Opaque,
    owner: isize,
    path: Option<PathBuf>,
    handler: Option<crate::Opaque>,
    /// Our own media player (Media Foundation's MFPlay) drawing into the same window, for
    /// audio / video that have no preview handler.
    player: Option<Player>,
}
struct Player {
    path: PathBuf,
    mp: crate::Opaque,
    /// Play was asked for before the file finished opening: the timer starts it.
    pending_play: u32,
}
const PLAY_TIMER: usize = 1;
thread_local! {
    static HOST : RefCell < Option < Host >> = const { RefCell::new(None) };
}
/// Automated UI tests set this: a test must never make a sound.
fn muted_for_tests() -> bool {
    std::env::var_os("RUSTYFILES_MUTE").is_some()
}
/// Whether `path` is audio / video our own player tries (Media Foundation decides in the end).
/// Linux has no Media Foundation player: rf-player plays these itself (with FFmpeg for what
/// FilmCraft can't), so the app never takes this path.
pub fn is_playable(_path: &Path) -> bool {
    false
}

type MfpCreate = unsafe extern "system" fn(
    crate::Opaque,
    crate::Opaque,
    crate::Opaque,
    *mut core::ffi::c_void,
    crate::Opaque,
    *mut *mut core::ffi::c_void,
) -> crate::Opaque;
/// Media Foundation is loaded on the first Play, not at start-up: linking MFPCreateMediaPlayer
/// statically made every launch load mfplay.dll and mfplat.dll. Also runs MFStartup once.
fn mf_create() -> Option<MfpCreate> {
    None
}
/// Plays `path` with our own player; `rect` (screen pixels) shows its video over the pane,
/// `None` plays without a visible window (audio). Re-pressing play on the same file resumes.
/// Call on the shell STA thread.
pub fn media_play(owner: isize, path: &Path, rect: Option<crate::RECT>) {}
fn current_player() -> Option<crate::Opaque> {
    None
}
/// Pauses our player. STA thread.
pub fn media_pause() {

}
fn toggle_play() {}
/// Jumps to `fraction` (0..1) of the length. STA thread.
pub fn media_seek(fraction: f64) {}
/// Playback state for the controls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaState {
    pub playing: bool,
    /// Seconds.
    pub position: f64,
    pub duration: f64,
}
/// The player's state, `None` without a player. STA thread.
pub fn media_state() -> Option<MediaState> {
    None
}
/// Moves the player's video (pane resized / window moved). STA thread.
pub fn media_reposition(rect: crate::RECT) {}
/// Stops and releases our player (the file is closed).
fn stop_player() {

}
fn host_window(owner: isize) -> Option<crate::Opaque> {
    None
}
fn unload(handler: Option<crate::Opaque>) {}
/// Theme colors for handlers that support it (dark mode text/background).
pub struct Colors {
    pub background: u32,
    pub text: u32,
}
/// Shows `path` over `rect` (screen pixels). Returns false if no handler could preview it.
/// Call on the shell STA thread.
pub fn show(owner: isize, path: &Path, rect: crate::RECT, colors: &Colors) -> bool {
    false
}
fn position(hwnd: crate::Opaque, rect: crate::Opaque) {}
fn load(
    hwnd: crate::Opaque,
    path: &Path,
    rect: crate::Opaque,
    colors: &Colors,
) -> Option<crate::Opaque> {
    None
}
/// 0xRRGGBB → COLORREF 0x00BBGGRR
fn bgr(c: u32) -> u32 {
    ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF)
}
/// Hides the host and unloads the handler (frees the document). Call on the STA thread.
pub fn hide() {}
/// Moves the visible host (window moved / pane resized) without reloading.
pub fn reposition(rect: crate::RECT) {}

/// Whether the system has a hardware video decoder for `codec` (Windows: Media Foundation,
/// for 4K and past-1080p60 video). Linux: no; DirectLook decodes on the CPU as before.
pub fn mf_decodes(_codec: &str) -> bool {
    false
}
