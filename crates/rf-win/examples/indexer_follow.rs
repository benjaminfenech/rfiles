//! The indexer service following changes through the change journal: index a temp folder,
//! change things in it, and time until searches see the changes.
//! `cargo run --release -p rf-win --example indexer_follow`

#[cfg(windows)]
#[path = "support/indexer_follow_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("indexer_follow needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
