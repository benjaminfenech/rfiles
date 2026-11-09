//! Can a normal (not elevated) process read a volume's change journal? Opens the drive's root
//! folder, asks for the journal, reads the latest records with the unprivileged control code
//! and resolves a parent folder id to a path.
//! `cargo run --release -p rf-win --example usn_probe -- C:\`

#[cfg(windows)]
#[path = "support/usn_probe_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("usn_probe needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
