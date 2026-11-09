//! Does listing with file ids (`GetFileInformationByHandleEx`, needed for the change journal)
//! make the index walk slower than `FindFirstFileEx`? Alternates the two, wall time and CPU.
//! `cargo run --release -p rf-win --example walk_ids -- <folder> <rounds>`

#[cfg(windows)]
#[path = "support/walk_ids_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("walk_ids needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
