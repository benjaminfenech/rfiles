//! Starts a copy with the journal on and kills the process part-way, like a crash or a power
//! cut would: `crashcopy <source> <dest folder> <journal folder> <ms before the crash>`.
//! The app's next start should offer to resume it.

#[cfg(windows)]
#[path = "support/crashcopy_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("crashcopy needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
