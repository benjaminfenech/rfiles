//! Shows the desktop's file chooser the way the app does and prints the choice:
//! `cargo run -p rf-linux --example pick -- open | folder | save`
#[cfg(unix)]
fn main() {
    use rf_linux::dialogs::{Pick, pick};
    let kind = std::env::args().nth(1).unwrap_or_default();
    let toml = [("Settings file", "*.toml")];
    let r = match kind.as_str() {
        "folder" => pick(0, "Choose a folder", Pick::Folder),
        "save" => pick(0, "Export settings", Pick::Save { filters: &toml, default_name: "rfiles-settings.toml" }),
        _ => pick(0, "Choose a picture", Pick::File { filters: &[("Pictures", "*.png;*.jpg")] }),
    };
    println!("{r:?}");
}

#[cfg(not(unix))]
fn main() {}
