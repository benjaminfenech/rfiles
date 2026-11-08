//! Copy engine benchmark: builds a test tree once (20 000 small files in 200 folders, 20 files of
//! 50 MB), then times `fastcopy::transfer` (Copy) into a fresh folder.
//!
//!   cargo run --release -p rf-win --example copybench [work-dir] [runs]
//!
//! The work dir defaults to %TEMP%\rf-copybench. Nothing outside it is touched.

#[cfg(windows)]
#[path = "support/copybench_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("copybench needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
