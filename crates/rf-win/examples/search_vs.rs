//! Searching subfolders, rustyfiles against File Explorer, the way people do it: both already
//! running (Explorer's shell; a rustyfiles with its search index ready), a search link sent to
//! each, the time until the results on screen stop changing for a second.
//! `cargo run --release -p rf-win --example search_vs -- <rustyfiles.exe> <folder> <query> <rounds>`

#[cfg(windows)]
#[path = "support/search_vs_win.rs"]
mod win;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("search_vs needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
