//! Opens and closes a playing video window many times (hidden, muted) and prints the process'
//! private memory and handles, to catch what a closed window leaves behind.
//! `cargo run --release -p rustdecoder --example soak_window -- <clip> <rounds>`

#[cfg(windows)]
#[path = "support/soak_window_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("soak_window needs Windows (Direct3D 11 and a Win32 window)");
    std::process::exit(1);
}
