//! Deneme penceresi: `cargo run --release -p rf-player --example play_window -- <video> [--sound]`
//! Videoyu küçük bir pencerede oynatır, bitince kapanır. Varsayılan sessizdir (NullSink).

#[cfg(windows)]
#[path = "support/play_window_win.rs"]
mod win;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("play_window needs Windows (Direct3D 11)");
    std::process::exit(1);
}
