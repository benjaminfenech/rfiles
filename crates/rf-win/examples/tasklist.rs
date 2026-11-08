//! Prints the windows a taskbar would show, grouped like taskbar buttons (read-only).
//!
//!   cargo run --release -p rf-win --example tasklist

#[cfg(windows)]
#[path = "support/tasklist_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("tasklist needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
