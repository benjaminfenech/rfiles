//! DirectLook's rustdecoder route on Linux: never taken. rustdecoder plays through Direct3D 11,
//! Media Foundation and WASAPI in a Win32 popup (`rustdecoder::window`, Windows only), so on
//! Linux [`enabled`] is always false and videos keep the routes `video.rs` has always used.
//! Same API as `rd_video.rs`, so the viewer compiles unchanged.

use std::path::Path;

pub fn enabled() -> bool {
    false
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub codec: &'static str,
}

pub fn probe(_path: &Path) -> Option<Probe> {
    None
}

pub struct Settings {
    pub volume: f32,
    pub muted: bool,
    pub looping: bool,
}

/// rustdecoder's window state, as far as the viewer reads it.
#[derive(Debug, Clone, Default)]
pub struct WindowState {
    pub playing: bool,
    pub ended: bool,
    pub position: f64,
    pub duration: f64,
    /// Size of the picture as shown.
    pub video_size: (u32, u32),
}

/// rustdecoder's window events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEvent {
    Click,
    DoubleClick,
    Wheel(i32),
    MouseMove,
    Leave,
    Closed,
}

/// Never made on Linux: [`RdVideo::open`] always fails.
pub struct RdVideo(());

impl RdVideo {
    pub fn open(_path: &Path, _owner: isize, _rect: [i32; 4], _s: &Settings) -> Result<RdVideo, String> {
        Err("rustdecoder plays on Windows only".into())
    }
    pub fn state(&self) -> WindowState {
        WindowState::default()
    }
    pub fn toggle(&self) {}
    pub fn seek_fraction(&self, _f: f64) {}
    pub fn set_volume(&self, _v: f32) {}
    pub fn set_muted(&self, _m: bool) {}
    pub fn set_looping(&self, _l: bool) {}
    pub fn reposition(&self, _rect: [i32; 4]) {}
    pub fn take_events(&self) -> Vec<WindowEvent> {
        Vec::new()
    }
}
