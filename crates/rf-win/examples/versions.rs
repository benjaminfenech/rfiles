//! Two builds of rustyfiles (and File Explorer) opening the same folder, measured the same way as
//! the speed test, taking turns: `cargo run --release -p rf-win --example versions --
//! <folder> <runs> <label=exe>...`. Each rustyfiles starts as an extra instance with its own
//! settings folder (the user's settings and running app are left alone).

#[cfg(windows)]
#[path = "support/versions_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("versions needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
