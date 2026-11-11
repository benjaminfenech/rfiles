//! Shows which icon theme is used and which files icons resolve to:
//! `cargo run -p rf-linux --example icon_lookup -- [ext…]`

fn main() {
    use rf_linux::xdg_icons::*;
    println!("theme: {}", current_theme_name());
    let exts: Vec<String> = std::env::args().skip(1).collect();
    let exts = if exts.is_empty() { ["png", "pdf", "rs", "mp4", "zip", "txt", "docx", "mp3", "unknownext"].map(String::from).to_vec() } else { exts };
    println!("folder -> {:?}", find_icon("folder", 32));
    for ext in exts {
        let mime = mime_for_ext(&ext);
        let names = mime.map(icon_names_for_mime).unwrap_or_default();
        let found = names.iter().find_map(|n| find_icon(n, 32).map(|p| (n.clone(), p)));
        println!("{ext:>10}  {:<28} -> {:?}", mime.unwrap_or("?"), found);
    }
}

