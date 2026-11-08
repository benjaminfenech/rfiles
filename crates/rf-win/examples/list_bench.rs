//! Directory listing speed, single thread, whole tree: FindFirstFileExW (large fetch) vs
//! GetFileInformationByHandleEx(FileFullDirectoryInfo) with big buffers.
//! `cargo run --release -p rf-win --example list_bench -- <dir>`

#[cfg(windows)]
#[path = "support/list_bench_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("list_bench needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
