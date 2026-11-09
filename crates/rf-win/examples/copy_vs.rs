//! Our copy engine against Windows' (IFileOperation) on the same sources, alternating which goes
//! first each round: `cargo run --release -p rf-win --example copy_vs -- <dest-dir> <rounds> <source>...`.

#[cfg(windows)]
#[path = "support/copy_vs_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("copy_vs needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
