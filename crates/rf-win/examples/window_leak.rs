//! Issue #3: memory left behind by each closed window while rustyfiles keeps running in the
//! background. Opens a window on `<folder>` through a second launch, closes it with WM_CLOSE,
//! and prints the primary process' private bytes / working set / handles after every close.
//! `cargo run --release -p rf-win --example window_leak -- <rustyfiles.exe> <folder> <windows>`

#[cfg(windows)]
#[path = "support/window_leak_win.rs"]
mod win;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("window_leak needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
