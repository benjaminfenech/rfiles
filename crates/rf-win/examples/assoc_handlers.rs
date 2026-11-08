//! Lists the "Open with" handlers Windows offers for an extension: `assoc_handlers .7z`.

#[cfg(windows)]
#[path = "support/assoc_handlers_win.rs"]
mod win;

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("assoc_handlers needs Windows (the Win32 platform layer)");
    std::process::exit(1);
}
