//! Size column speed: every sub-folder of a directory, one after another (old way) vs one batch.
//! `cargo run --release -p rf-win --example dirsize_bench -- <dir>`

#[cfg(windows)]
#[path = "support/dirsize_bench_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("dirsize_bench needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
