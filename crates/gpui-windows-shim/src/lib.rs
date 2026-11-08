//! Stand-in for the rustyfiles-specific API of our GPUI Windows fork
//! (github.com/gelenmedya/gpui-pre-windows) on platforms where GPUI uses its own backend.
//! Same names and types, no effect: rf-app's call sites stay as they are.

use std::sync::atomic::AtomicBool;

/// Windows: keep the window drawn as active while a popup has focus. No effect here.
pub static KEEP_ACTIVE_LOOK: AtomicBool = AtomicBool::new(false);

/// Windows: smooth wheel scrolling in the fork's event loop. No effect here.
pub static SMOOTH_SCROLL: AtomicBool = AtomicBool::new(true);

/// Windows: the drop effect shown while dragging into another app. No effect here.
pub mod dnd_hint {
    pub const COPY: u32 = 1;
    pub const MOVE: u32 = 2;

    pub fn set(_effect: u32) {}
}

/// Windows: the GPU/CPU hot-swap renderer. GPUI's Linux backend always draws on the GPU
/// (wgpu), so this reports a fixed "GPU" and ignores mode changes.
pub mod hybrid {
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    #[repr(u8)]
    pub enum RendererMode {
        Auto = 0,
        Gpu = 1,
        Cpu = 2,
    }

    impl RendererMode {
        /// Parses `auto` / `gpu` / `cpu` (case-insensitive).
        pub fn parse(s: &str) -> Option<Self> {
            match s.trim().to_ascii_lowercase().as_str() {
                "auto" => Some(Self::Auto),
                "gpu" | "hardware" | "hw" => Some(Self::Gpu),
                "cpu" | "warp" | "software" | "sw" => Some(Self::Cpu),
                _ => None,
            }
        }
    }

    #[derive(Copy, Clone, Debug)]
    pub struct LoadSnapshot {
        pub gpu_percent: Option<f32>,
        pub cpu_percent: Option<f32>,
        pub last_frame_ms: f32,
    }

    pub fn mode() -> RendererMode {
        RendererMode::Gpu
    }

    pub fn set_mode(_mode: RendererMode) {}

    pub fn switch_count() -> u32 {
        0
    }

    pub fn current_backend_label() -> &'static str {
        "GPU"
    }

    pub fn load_snapshot() -> LoadSnapshot {
        LoadSnapshot { gpu_percent: None, cpu_percent: None, last_frame_ms: 0.0 }
    }
}
