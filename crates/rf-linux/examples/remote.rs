//! Opens a network location the way the app does and lists it:
//! `cargo run -p rf-linux --example remote -- smb://server/share | sftp://host/path | ftp://…`
#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_default();
    let dir = rf_linux::remote::mount(&url)?;
    println!("{url} → {}", dir.display());
    for e in std::fs::read_dir(&dir)? {
        println!("  {}", e?.file_name().to_string_lossy());
    }
    Ok(())
}

#[cfg(not(unix))]
fn main() {}
