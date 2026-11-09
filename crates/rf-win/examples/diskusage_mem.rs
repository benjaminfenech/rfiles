//! Walks a folder with the disk usage engine and reports time, entries and memory:
//! `cargo run --profile fast -p rf-win --example diskusage_mem -- C:\Windows`

#[cfg(windows)]
#[path = "support/diskusage_mem_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("diskusage_mem needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
