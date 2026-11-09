//! Subfolder search, index against our own walk:
//! `cargo run --release -p rf-win --example search_bench -- <folder> <query> [runs]`.
//! Prints, per method, the time to the first hit and to the last, and the hit count.

#[cfg(windows)]
#[path = "support/search_bench_win.rs"]
mod win;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("search_bench needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
