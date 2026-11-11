The Linux port of [rustyfiles](https://github.com/gelenmedya/rustyfiles), built from upstream **0.6.0**. It runs on Wayland and X11.

## What's new since 0.5.2-linux.1

- **Network section in the sidebar.** Network shares (SMB, NFS, SSHFS…) now have their own section instead of sitting among the drives. Every share in `/etc/fstab` is listed, mounted or not, in the file's order.
- **Automounted shares no longer come and go.** Shares with `x-systemd.automount` used to appear only after another app had opened them, and a share that mounted later never showed up. Now they're always there, and opening one mounts it.
- **Day/month/year dates.** Settings → Date format has a new option that shows `05/11/2026 13:45`.
- **New Properties window** (upstream 0.6.0), with tabs: General ("Opens with" can be changed through your desktop's app chooser), Details, Hashes, and a drive page for `/` and other mount points. Windows-only tabs are hidden.
- From upstream 0.6.0: DirectLook's APK card draws vector and adaptive app icons, and menus open at their own size on the window's monitor.
- A new, Linux-only [README](https://github.com/benjaminfenech/rustyfiles-linux#readme).

## Install

| System | File | Command |
| --- | --- | --- |
| Debian 12+, Ubuntu 22.04+ | `rustyfiles_…_amd64.deb` / `_arm64.deb` | `sudo apt install ./rustyfiles_*.deb` |
| Fedora | `rustyfiles-….x86_64.rpm` / `.aarch64.rpm` | `sudo dnf install ./rustyfiles-*.rpm` |
| openSUSE | the same `.rpm` | `sudo zypper install ./rustyfiles-*.rpm` |
| Arch, CachyOS, Manjaro | `rustyfiles-git-…-x86_64.pkg.tar.zst` | `sudo pacman -U ./rustyfiles-git-*.pkg.tar.zst` |
| Any distribution | `rustyfiles-….x86_64.flatpak` / `.aarch64.flatpak` | `flatpak install --user ./rustyfiles-*.flatpak` |

Installing a .deb, .rpm or Arch package over 0.5.2-linux.1 upgrades it in place, and your settings stay. For the Flatpak, add `--reinstall` if it says the app is already installed; it fetches its runtime from Flathub on first install.

Optional extras for previews and drives: poppler (PDFs), libavif / libheif tools (AVIF, HEIC), FFmpeg (older video formats), udisks2 (mounting drives), kio-fuse or gvfs (network locations), git (git status). The [README](https://github.com/benjaminfenech/rustyfiles-linux#optional-extras) lists the package names for each distribution.

This release has no Windows build: for Windows, use [upstream's releases](https://github.com/gelenmedya/rustyfiles/releases).
