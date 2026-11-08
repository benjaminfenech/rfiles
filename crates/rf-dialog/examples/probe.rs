//! Loads rustyfiles_dialog.dll straight from the build folder (no registry) and shows its Open,
//! Save or folder dialog like an app would; prints what came back.
//!   probe open|save|folder <start folder>

#[cfg(windows)]
#[path = "support/probe_win.rs"]
mod win;

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("probe needs Windows (COM file dialogs)");
    std::process::exit(1);
}
