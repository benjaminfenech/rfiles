//! Resumes every interrupted job in a journal folder (the app's "Resume" without the app):
//! `resumecopy <journal folder>`. Prints what it found and how long resuming took.

#[cfg(windows)]
#[path = "support/resumecopy_win.rs"]
mod win;

#[cfg(windows)]
fn main() {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("resumecopy needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
