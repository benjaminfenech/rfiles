//! Turns "default file manager" on or off for the user in `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME`
//! (point those at a scratch folder to try it safely), or shows its state:
//! `cargo run -p rf-linux --example default_fm -- enable <exe> | disable | status`

#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    use rf_linux::default_fm::{CLASSES, disable, enable, is_enabled, registered_exe};
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("enable") => enable(args.get(1).expect("exe"), CLASSES)?,
        Some("disable") => disable(CLASSES)?,
        _ => {}
    }
    println!("default file manager: {} ({:?})", is_enabled(CLASSES), registered_exe(CLASSES));
    Ok(())
}

#[cfg(not(unix))]
fn main() {
    eprintln!("this example is for the Linux build (rf-linux)");
}
