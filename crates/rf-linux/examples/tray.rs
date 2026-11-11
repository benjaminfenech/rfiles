//! Shows the tray icon for `secs` seconds (default 10) and prints clicks and menu choices; with
//! `notify`, sends a notification first:
//! `cargo run -p rf-linux --example tray -- [notify] [secs]`
fn main() {
    use rf_linux::tray;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(10);
    tray::set_handler(|e| println!("event: {e:?}"));
    tray::show(tray::Labels { tooltip: "rfiles".into(), open: "Open rfiles".into(), new_window: "New window".into(), settings: "Settings".into(), quit: "Quit".into() });
    if args.iter().any(|a| a == "notify") {
        tray::notify("Operation complete", "Copying 3 items to Downloads");
    }
    std::thread::sleep(std::time::Duration::from_secs(secs));
    tray::hide();
    std::thread::sleep(std::time::Duration::from_millis(300));
}

