//! Per-thread CPU time of a process, with thread names:
//! `cargo run --release -p rf-win --example thread_cpu -- <pid>`.

#[cfg(windows)]
#[path = "support/thread_cpu_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("thread_cpu needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
