//! Lists the apps "Open with" offers for files, and the command each would run (nothing is
//! launched): `cargo run -p rf-linux --example open_with -- <file>…`

fn main() {
    use rf_linux::xdg_apps::*;
    let files: Vec<std::path::PathBuf> = std::env::args().skip(1).map(Into::into).collect();
    let Some(first) = files.first() else {
        eprintln!("usage: open_with <file>…");
        return;
    };
    let mime = mime_for_path(first);
    println!("type: {mime}  (with parents: {:?})", with_parents(&mime));
    println!("default: {:?}", default_app(&mime).map(|a| &a.name));
    for app in apps_for(&mime) {
        println!("  {:<28} {:?}", app.name, command_lines(app, &files));
    }
    println!("other apps: {}", all_apps().len());
}

