//! The Settings speed test without the UI: `cargo run --release -p rf-app --example bench_cli
//! -- [quick]`. Needs target/release/rfiles.exe built. Prints the results as JSON.

#[cfg(windows)]
#[path = "support/bench_cli_win.rs"]
mod win;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("bench_cli races File Explorer: Windows only");
    std::process::exit(1);
}
