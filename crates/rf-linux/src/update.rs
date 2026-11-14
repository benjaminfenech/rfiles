//! Updates from GitHub Releases: is there a newer release, which of its files fits this
//! install (the .deb, .rpm, Arch package or Flatpak it came from, for this CPU), a download
//! checked against the release's signed checksums, and the install, done the way the user
//! would do it by hand (`apt-get`, `dnf`/`zypper`, `pacman` through `pkexec`, which asks for
//! the password; `flatpak install` needs none).
//!
//! Every release carries `SHA256SUMS` and `SHA256SUMS.sig`, an Ed25519 signature made in CI
//! with a key kept as a repository secret. Its public half, [`RELEASE_KEY`], is built into
//! rfiles. A package is installed only when its SHA-256 is in a list that key signed: a
//! release or download swapped on GitHub's side, without the key, is refused.
//!
//! Everything here blocks (network, package tools): worker threads only.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

/// Where the releases are.
pub const REPO: &str = "benjaminfenech/rfiles";

/// A published release.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// "0.8.0" (the tag without its "v").
    pub version: String,
    /// Its page on GitHub (the notes).
    pub url: String,
    pub assets: Vec<Asset>,
}

/// One file of a release.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex, from the release's signed `SHA256SUMS` ([`verify_signed`]); `None` until
    /// then, or when the list doesn't name the file. [`download`] refuses a file without it.
    pub sha256: Option<String>,
}

/// How this copy of rfiles was installed, which decides the file an update takes and how it
/// is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    Deb,
    Rpm,
    Arch,
    Flatpak,
    /// Built from source, or a package manager we don't know: updates are only announced.
    Other,
}

fn user_agent() -> String {
    format!("rfiles/{} (updater)", env!("CARGO_PKG_VERSION"))
}

/// The latest published release (drafts and pre-releases are never "latest").
pub fn latest() -> Result<Release> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
    let mut res = agent
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("User-Agent", user_agent())
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("GitHub")?;
    let text = res.body_mut().read_to_string().context("GitHub")?;
    parse_release(&text)
}

fn parse_release(json: &str) -> Result<Release> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow!("no tag in the release"))?;
    let assets = v["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(Asset {
                        name: x["name"].as_str()?.to_string(),
                        url: x["browser_download_url"].as_str()?.to_string(),
                        size: x["size"].as_u64().unwrap_or(0),
                        // only a signed checksum counts (verify_signed), not GitHub's own digest
                        sha256: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        url: v["html_url"].as_str().unwrap_or_default().to_string(),
        assets,
    })
}

/// (major, minor, patch) of "v0.8.0", "0.8.0", "0.6.0-linux.1" (what follows `-` or `+` is
/// ignored).
fn parts(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>());
    let t = (it.next()?.ok()?, it.next().unwrap_or(Ok(0)).ok()?, it.next().unwrap_or(Ok(0)).ok()?);
    Some(t)
}

/// Whether `candidate` is a newer version than `current`.
pub fn newer(candidate: &str, current: &str) -> bool {
    matches!((parts(candidate), parts(current)), (Some(a), Some(b)) if a > b)
}

/// How this copy was installed (asked once: the package tools are run).
pub fn install_kind() -> Install {
    static KIND: OnceLock<Install> = OnceLock::new();
    *KIND.get_or_init(|| {
        if crate::sandbox::is_flatpak() {
            return Install::Flatpak;
        }
        let Ok(exe) = std::env::current_exe() else { return Install::Other };
        let owns = |prog: &str, args: &[&str]| {
            let mut c = crate::sandbox::tool(prog, &[]);
            c.args(args).arg(&exe);
            crate::tool::run(c, Duration::from_secs(10)).is_ok_and(|r| r.ok)
        };
        if owns("dpkg-query", &["-S"]) {
            Install::Deb
        } else if owns("rpm", &["-qf"]) {
            Install::Rpm
        } else if owns("pacman", &["-Qo"]) {
            Install::Arch
        } else {
            Install::Other
        }
    })
}

/// The release file for `kind` on CPU `arch` (`std::env::consts::ARCH`: "x86_64", "aarch64").
pub fn pick_asset<'a>(release: &'a Release, kind: Install, arch: &str) -> Option<&'a Asset> {
    let deb_arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => a,
    };
    release.assets.iter().find(|a| {
        let n = a.name.as_str();
        match kind {
            Install::Deb => n.starts_with("rfiles_") && n.ends_with(&format!("_{deb_arch}.deb")),
            Install::Rpm => n.starts_with("rfiles-") && n.ends_with(&format!(".{arch}.rpm")),
            Install::Arch => n.starts_with("rfiles-git-") && n.ends_with(&format!("-{arch}.pkg.tar.zst")),
            Install::Flatpak => n.starts_with("rfiles-") && n.ends_with(&format!(".{arch}.flatpak")),
            Install::Other => false,
        }
    })
}

/// The Ed25519 public key release checksums are signed with (raw 32 bytes; the same key as
/// `packaging/linux/release-key.pub.pem`).
pub const RELEASE_KEY: [u8; 32] = include!("../../../packaging/linux/release-key.pub.bytes");

const SUMS: &str = "SHA256SUMS";
const SUMS_SIG: &str = "SHA256SUMS.sig";

/// Fetches the release's `SHA256SUMS` and its signature, checks the signature against
/// [`RELEASE_KEY`] and returns the release with each listed file's SHA-256 filled in.
/// Fails when the release isn't signed or the signature doesn't match.
pub fn verify_signed(release: &Release) -> Result<Release> {
    let find = |name: &str| release.assets.iter().find(|a| a.name == name);
    let (Some(sums), Some(sig)) = (find(SUMS), find(SUMS_SIG)) else {
        bail!("release {} isn't signed: not installing it", release.version);
    };
    let sums_text = fetch_small(&sums.url)?;
    let sig = fetch_small(&sig.url)?;
    let hashes = check_sums(&sums_text, &sig, &RELEASE_KEY)?;
    let mut r = release.clone();
    for a in &mut r.assets {
        a.sha256 = hashes.get(&a.name).cloned();
    }
    Ok(r)
}

/// Verifies `sig` over `sums` with `key` and parses `sha256sum` lines (`<hex>  <name>`).
fn check_sums(sums: &[u8], sig: &[u8], key: &[u8; 32]) -> Result<std::collections::HashMap<String, String>> {
    use ring::signature::{ED25519, UnparsedPublicKey};
    UnparsedPublicKey::new(&ED25519, key)
        .verify(sums, sig)
        .map_err(|_| anyhow!("the release's signature doesn't match rfiles' release key: not installing it"))?;
    let text = std::str::from_utf8(sums).context("SHA256SUMS")?;
    Ok(text
        .lines()
        .filter_map(|l| {
            let (hash, name) = l.split_once(' ')?;
            let name = name.trim_start_matches([' ', '*']);
            (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| (name.to_string(), hash.to_ascii_lowercase()))
        })
        .collect())
}

/// A small release file (the checksums, their signature), at most 1 MiB.
fn fetch_small(url: &str) -> Result<Vec<u8>> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into();
    let mut res = agent.get(url).header("User-Agent", user_agent()).call().context("download")?;
    let mut out = Vec::new();
    res.body_mut().as_reader().take(1 << 20).read_to_end(&mut out).context("download")?;
    Ok(out)
}

/// Where updates are downloaded: the cache folder (inside Flatpak the host's, so the host's
/// `flatpak` and `pkexec` can read the file).
pub fn download_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("rfiles").join("updates")
}

/// Downloads `asset` into `dir` (older downloads there are removed) and checks its SHA-256.
/// `progress(done, total)` is called as it arrives; `cancel` stops it.
pub fn download(asset: &Asset, dir: &Path, progress: &dyn Fn(u64, u64), cancel: &dyn Fn() -> bool) -> Result<PathBuf> {
    let want = asset.sha256.as_deref().ok_or_else(|| anyhow!("{} isn't in the release's signed checksums: not installing it", asset.name))?;
    if asset.name.contains(['/', '\\']) || asset.name.starts_with('.') {
        bail!("unexpected file name {:?}", asset.name);
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir)?;
    let dest = dir.join(&asset.name);
    let part = dir.join(format!("{}.part", asset.name));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into();
    let res = agent.get(&asset.url).header("User-Agent", user_agent()).call().context("download")?;
    let mut body = res.into_body().into_reader();
    let mut file = std::fs::File::create(&part)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 256 << 10];
    let mut done = 0u64;
    loop {
        if cancel() {
            drop(file);
            let _ = std::fs::remove_file(&part);
            bail!("cancelled");
        }
        let n = body.read(&mut buf).context("download")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hash.update(&buf[..n]);
        done += n as u64;
        progress(done, asset.size.max(done));
    }
    file.sync_all()?;
    drop(file);
    let got = rf_core::uri::hex(&hash.finalize());
    if got != want || (asset.size > 0 && done != asset.size) {
        let _ = std::fs::remove_file(&part);
        bail!("the download of {} is damaged (checksum doesn't match): not installing it", asset.name);
    }
    std::fs::rename(&part, &dest)?;
    Ok(dest)
}

/// Whether the Flatpak is a system-wide install (else a per-user one).
fn flatpak_is_system() -> bool {
    std::fs::read_to_string("/.flatpak-info").is_ok_and(|s| s.lines().any(|l| l.trim().starts_with("app-path=/var/lib/flatpak/")))
}

/// The command that installs `file` for `kind`; `None` for [`Install::Other`].
fn install_command(kind: Install, file: &Path, have: &dyn Fn(&str) -> bool, flatpak_system: bool) -> Option<Command> {
    let pk = |args: &[&str]| {
        let mut c = crate::sandbox::tool("pkexec", &[]);
        c.args(args).arg(file);
        c
    };
    Some(match kind {
        Install::Deb if have("apt-get") => pk(&["apt-get", "install", "-y", "--allow-downgrades"]),
        Install::Deb => pk(&["dpkg", "-i"]),
        Install::Rpm if have("dnf") => pk(&["dnf", "install", "-y"]),
        Install::Rpm if have("zypper") => pk(&["zypper", "--non-interactive", "install", "--allow-unsigned-rpm"]),
        Install::Rpm => pk(&["rpm", "-U"]),
        Install::Arch => pk(&["pacman", "-U", "--noconfirm"]),
        Install::Flatpak => {
            let mut c = crate::sandbox::tool("flatpak", &[]);
            c.args(["install", if flatpak_system { "--system" } else { "--user" }, "--noninteractive", "--reinstall", "--bundle"]).arg(file);
            c
        }
        Install::Other => return None,
    })
}

/// Installs a downloaded update. `pkexec` shows the desktop's password prompt; a dismissed
/// prompt is an error saying so.
pub fn install(kind: Install, file: &Path) -> Result<()> {
    let have = |p: &str| crate::sandbox::in_path(p);
    if kind != Install::Flatpak && !have("pkexec") {
        bail!("pkexec isn't installed: install {} by hand", file.display());
    }
    let cmd = install_command(kind, file, &have, flatpak_is_system()).ok_or_else(|| anyhow!("this copy wasn't installed from a package"))?;
    let shown = format!("{} {}", cmd.get_program().to_string_lossy(), cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    log::info!("update: {shown}");
    let r = crate::tool::run(cmd, Duration::from_secs(30 * 60))?;
    if r.ok {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&r.stderr);
    let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    // pkexec: 126 = the password prompt was dismissed, 127 = not authorized
    if last.contains("dismissed") || last.contains("Not authorized") || last.contains("Request dismissed") {
        bail!("the password prompt was cancelled");
    }
    bail!("{}", if last.is_empty() { "the install failed" } else { last })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "tag_name": "v0.8.0", "html_url": "https://github.com/benjaminfenech/rfiles/releases/tag/v0.8.0",
        "assets": [
            {"name": "rfiles_0.8.0-1_amd64.deb", "browser_download_url": "https://x/a.deb", "size": 10, "digest": "sha256:ABCD"},
            {"name": "rfiles_0.8.0-1_arm64.deb", "browser_download_url": "https://x/b.deb", "size": 10, "digest": null},
            {"name": "rfiles-0.8.0-1.x86_64.rpm", "browser_download_url": "https://x/c.rpm", "size": 10},
            {"name": "rfiles-0.8.0-1.aarch64.rpm", "browser_download_url": "https://x/d.rpm", "size": 10},
            {"name": "rfiles-git-0.8.0.r400.gabc1234-1-x86_64.pkg.tar.zst", "browser_download_url": "https://x/e", "size": 10},
            {"name": "rfiles-0.8.0.x86_64.flatpak", "browser_download_url": "https://x/f", "size": 10},
            {"name": "rfiles-0.8.0.aarch64.flatpak", "browser_download_url": "https://x/g", "size": 10}
        ]
    }"#;

    #[test]
    fn reads_a_release() {
        let r = parse_release(SAMPLE).unwrap();
        assert_eq!(r.version, "0.8.0");
        assert_eq!(r.assets.len(), 7);
        // GitHub's digest isn't trusted: only verify_signed fills it in
        assert!(r.assets.iter().all(|a| a.sha256.is_none()));
    }

    #[test]
    fn versions() {
        assert!(newer("v0.8.0", "0.7.0"));
        assert!(newer("0.7.1", "0.7.0"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("v0.7.0", "0.7.0"));
        assert!(!newer("0.6.0-linux.1", "0.7.0"));
        assert!(!newer("garbage", "0.7.0"));
        assert!(newer("v0.10.0", "0.9.0"), "numbers, not text");
    }

    #[test]
    fn picks_the_file_for_this_install() {
        let r = parse_release(SAMPLE).unwrap();
        let name = |k, a| pick_asset(&r, k, a).map(|x| x.name.as_str());
        assert_eq!(name(Install::Deb, "x86_64"), Some("rfiles_0.8.0-1_amd64.deb"));
        assert_eq!(name(Install::Deb, "aarch64"), Some("rfiles_0.8.0-1_arm64.deb"));
        assert_eq!(name(Install::Rpm, "aarch64"), Some("rfiles-0.8.0-1.aarch64.rpm"));
        assert_eq!(name(Install::Arch, "x86_64"), Some("rfiles-git-0.8.0.r400.gabc1234-1-x86_64.pkg.tar.zst"));
        assert_eq!(name(Install::Arch, "aarch64"), None);
        assert_eq!(name(Install::Flatpak, "x86_64"), Some("rfiles-0.8.0.x86_64.flatpak"));
        assert_eq!(name(Install::Other, "x86_64"), None);
    }

    #[test]
    fn install_commands() {
        let file = Path::new("/c/u/rfiles.pkg");
        let line = |k, have: &dyn Fn(&str) -> bool, sys| {
            install_command(k, file, have, sys).map(|c| {
                std::iter::once(c.get_program().to_string_lossy().into_owned()).chain(c.get_args().map(|a| a.to_string_lossy().into_owned())).collect::<Vec<_>>().join(" ")
            })
        };
        let all = |_: &str| true;
        let none = |_: &str| false;
        let zypper = |p: &str| p == "zypper";
        assert_eq!(line(Install::Deb, &all, false).as_deref(), Some("pkexec apt-get install -y --allow-downgrades /c/u/rfiles.pkg"));
        assert_eq!(line(Install::Deb, &none, false).as_deref(), Some("pkexec dpkg -i /c/u/rfiles.pkg"));
        assert_eq!(line(Install::Rpm, &all, false).as_deref(), Some("pkexec dnf install -y /c/u/rfiles.pkg"));
        assert_eq!(line(Install::Rpm, &zypper, false).as_deref(), Some("pkexec zypper --non-interactive install --allow-unsigned-rpm /c/u/rfiles.pkg"));
        assert_eq!(line(Install::Arch, &all, false).as_deref(), Some("pkexec pacman -U --noconfirm /c/u/rfiles.pkg"));
        assert_eq!(line(Install::Flatpak, &all, true).as_deref(), Some("flatpak install --system --noninteractive --reinstall --bundle /c/u/rfiles.pkg"));
        assert_eq!(line(Install::Other, &all, false), None);
    }

    #[test]
    fn refuses_files_without_a_checksum() {
        let a = Asset { name: "rfiles_0.8.0-1_amd64.deb".into(), url: "https://invalid.invalid/x".into(), size: 1, sha256: None };
        let dir = std::env::temp_dir().join(format!("rf-update-{}", std::process::id()));
        let e = download(&a, &dir, &|_, _| {}, &|| false).unwrap_err();
        assert!(e.to_string().contains("signed checksums"), "{e}");
    }

    #[test]
    fn signed_sums() {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let key: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
        let sums = format!("{}  rfiles_0.9.1-1_amd64.deb\n{}  rfiles-0.9.1.x86_64.flatpak\n", "ab".repeat(32), "CD".repeat(32));
        let sig = pair.sign(sums.as_bytes());
        let got = check_sums(sums.as_bytes(), sig.as_ref(), &key).unwrap();
        assert_eq!(got["rfiles_0.9.1-1_amd64.deb"], "ab".repeat(32));
        assert_eq!(got["rfiles-0.9.1.x86_64.flatpak"], "cd".repeat(32));
        // a changed list, or another key: refused
        let swapped = sums.replace("ab", "ef");
        assert!(check_sums(swapped.as_bytes(), sig.as_ref(), &key).is_err());
        assert!(check_sums(sums.as_bytes(), sig.as_ref(), &RELEASE_KEY).is_err());
    }

    /// The built-in key is the one in packaging/linux/release-key.pub.pem (CI checks its
    /// signatures against that file).
    #[test]
    fn built_in_key_matches_the_pem() {
        let pem = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packaging/linux/release-key.pub.pem");
        let Ok(out) = Command::new("openssl").args(["pkey", "-pubin", "-in", pem, "-outform", "DER"]).output() else { return };
        assert!(out.status.success());
        assert_eq!(out.stdout[out.stdout.len() - 32..], RELEASE_KEY);
    }
}
