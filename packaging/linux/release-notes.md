Network shares no longer freeze rfiles, encrypted drives open, and the quick search gets a real global shortcut. This is also the first release the 0.8.0 updater can install by itself.

## What's new since 0.8.0

- **Offline network shares don't freeze the window.** Opening an `/etc/fstab` share (`x-systemd.automount`) while away from its network froze rfiles until the mount timed out. Now the window stays responsive, and rfiles checks the server first: if it doesn't answer, the folder says "Can't reach <server>" within 1.5 seconds. The Recycle Bin and Recent files no longer wait on such shares either.
- **Offline shares are marked.** In the sidebar and on Home, a share whose server can't be reached is dimmed and marked "Offline". The mark clears by itself once you're back on its network.
- **Encrypted drives.** LUKS volumes now show up (with a lock) and ask for their passphrase when opened; a wrong passphrase asks again. Unmounting or ejecting locks them again.
- **Eject button.** USB sticks, SD cards and discs have an eject button on their sidebar row.
- **Copy progress on the taskbar icon** for KDE Plasma, Ubuntu Dock / Dash to Dock and Plank.
- **Global shortcut for quick search.** Settings → Quick search → "Global shortcut" asks your desktop for one through the desktop portal: KDE Plasma and GNOME 48+ ask you to confirm the key, Hyprland binds it in its config (see the README).
- **Quick search floats on tiling desktops.** On Wayland it opens as an overlay, so Hyprland and Sway don't tile it, and it sits a little below the top of the screen instead of at its very edge.
- **Clearer folder errors.** Errors are in plain English instead of "Listeleme hatası (kod 19)".

**Updating:** from 0.8.0, Settings → About → Install (or the status bar's "rfiles 0.8.1 available"). From 0.7.0 or older, install by hand once as below.

## Install

| System | File | Command |
| --- | --- | --- |
| Debian 12+, Ubuntu 22.04+ | `rfiles_…_amd64.deb` / `_arm64.deb` | `sudo apt install ./rfiles_*.deb` |
| Fedora | `rfiles-….x86_64.rpm` / `.aarch64.rpm` | `sudo dnf install ./rfiles-*.rpm` |
| openSUSE | the same `.rpm` | `sudo zypper install ./rfiles-*.rpm` |
| Arch, CachyOS, Manjaro | `rfiles-git-…-x86_64.pkg.tar.zst` | `sudo pacman -U ./rfiles-git-*.pkg.tar.zst` |
| Any distribution | `rfiles-….x86_64.flatpak` / `.aarch64.flatpak` | `flatpak install --user ./rfiles-*.flatpak` |

Optional extras for previews and drives: poppler (PDFs), libavif / libheif tools (AVIF, HEIC), FFmpeg (older video formats), udisks2 (mounting drives), kio-fuse or gvfs (network locations), git (git status). The [README](https://github.com/benjaminfenech/rfiles#optional-extras) lists the package names for each distribution.

There is no Windows build. For Windows, use [upstream rustyfiles](https://github.com/gelenmedya/rustyfiles/releases).
