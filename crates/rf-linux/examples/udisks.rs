//! Tries the UDisks2 calls: `list | mount <device> | unmount <mount point> | eject <mount point>`.
fn main() -> anyhow::Result<()> {
    use rf_linux::udisks;
    use std::path::Path;
    let args: Vec<String> = std::env::args().skip(1).collect();
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("list") | None, _) => {
            for v in udisks::volumes()? {
                println!(
                    "{:<14} {:<6} {:>14} {:<16} removable={} optical={} ignore={} mounted={:?}",
                    v.device.display(),
                    v.fs,
                    v.size,
                    v.label,
                    v.removable,
                    v.optical,
                    v.ignore,
                    v.mount_points
                );
            }
        }
        (Some("mount"), Some(d)) => println!("{}", udisks::mount(Path::new(d))?.display()),
        (Some("unmount"), Some(r)) => udisks::unmount(Path::new(r))?,
        (Some("eject"), Some(r)) => udisks::eject(Path::new(r))?,
        _ => anyhow::bail!("usage: list | mount <device> | unmount <mount point> | eject <mount point>"),
    }
    Ok(())
}

