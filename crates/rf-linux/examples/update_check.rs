//! The updater by hand: `cargo run -p rf-linux --example update_check -- [--download]`.
//! Prints the latest release, how this copy counts as installed, and the file it would take;
//! `--download` also downloads and checks that file against the signed checksums (into a temporary folder, not installed).

fn main() -> anyhow::Result<()> {
    use rf_linux::update;
    let r = update::latest()?;
    let r = match update::verify_signed(&r) {
        Ok(signed) => {
            println!("signature: good");
            signed
        }
        Err(e) => {
            println!("signature: {e:#}");
            r
        }
    };
    let kind = update::install_kind();
    let arch = std::env::consts::ARCH;
    println!("latest {} ({}), {} files", r.version, r.url, r.assets.len());
    println!("this copy: {kind:?} on {arch}; newer than {}: {}", env!("CARGO_PKG_VERSION"), update::newer(&r.version, env!("CARGO_PKG_VERSION")));
    // an example's own exe belongs to no package: show what each kind would take
    for k in [update::Install::Deb, update::Install::Rpm, update::Install::Arch, update::Install::Flatpak] {
        println!("  {k:?}: {:?}", update::pick_asset(&r, k, arch).map(|a| (&a.name, a.size, a.sha256.is_some())));
    }
    if std::env::args().any(|a| a == "--download") {
        let a = update::pick_asset(&r, update::Install::Flatpak, arch).expect("a Flatpak for this CPU");
        let dir = std::env::temp_dir().join("rfiles-update-check");
        let f = update::download(a, &dir, &|d, t| eprint!("\r{} / {} KB", d >> 10, t >> 10), &|| false)?;
        println!("\ndownloaded and verified: {}", f.display());
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}
