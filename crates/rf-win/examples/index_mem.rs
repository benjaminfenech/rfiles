//! Memory and speed of the name index on a real file: load (converting an older format), search,
//! save in the current format and load that again.
//! `cargo run --release -p rf-win --example index_mem -- <file.rfidx> <pattern>`

#[cfg(windows)]
#[path = "support/index_mem_win.rs"]
mod win;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("index_mem needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
