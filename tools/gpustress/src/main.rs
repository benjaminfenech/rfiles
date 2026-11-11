//! GPU load generator for testing the rfiles hybrid renderer.
//!
//! `gpustress --seconds N --intensity 0..100 [--mode ps|cs] [--no-pdh]`
//!
//! * `ps` (default): full-screen triangle with a heavy pixel shader into an offscreen render
//!   target. Loads the `engtype_3D` engine the way a game does.
//! * `cs`: heavy compute shader dispatches writing a UAV buffer (D3D11 may schedule this on the
//!   3D engine or on a separate Compute engine depending on the driver; the PDH output shows it).
//!
//! Intensity is a duty cycle over 100 ms windows. Hard time limit: default 20 s, max 120 s.
//! Ctrl+C stops cleanly.

#[cfg(windows)]
#[path = "win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("gpustress needs Windows (Direct3D 11)");
    std::process::exit(1);
}
