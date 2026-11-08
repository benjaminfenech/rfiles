//! Small-file copy micro-benchmark: CopyFile2 vs our own read/write copy (times + attributes,
//! alternate streams checked), on the copybench tree's small files.
//!
//!   cargo run --release -p rf-win --example smallcopy -- <copybench work dir> [threads]

#[cfg(windows)]
#[path = "support/smallcopy_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("smallcopy needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
