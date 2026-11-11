The first release under the new name: **rustyfiles for Linux is now rfiles**, a project of its own. It began as the Linux port of [rustyfiles](https://github.com/gelenmedya/rustyfiles) by Özer Güzel, stopped following it after upstream 0.6.0, and is now Linux-only. It runs on Wayland and X11.

## What's new since 0.6.0-linux.1

- **New name: rfiles.** The command is `rfiles` and the Flatpak is `io.github.benjaminfenech.rfiles`. Installing over the old package keeps everything: settings, tags, pins and the session move over on first start, and so do "default file manager" and "start at login" if you had them on. Upgrade steps are under Install below.
- **Recent files on Home.** The Recent section shows the files you opened recently in any app (the desktop's `recently-used.xbel`). Before, it was always empty on Linux.
- **The date format applies everywhere.** Properties, the copy-conflict dialog and DirectLook's info and email cards now use the format chosen in Settings → Date format (for example day/month/year), not just the file list.
- **No prompts.** rfiles never asks for a GitHub star or a donation. Settings → About still has the links.
- **Menus show only what works.** "Create shortcut" and "Paste shortcut" made Windows `.lnk` files and did nothing on Linux; they're gone.
- **Leaner.** Upstream's Windows code is removed, about 66,000 lines in all: the Windows platform layer, the Open/Save dialog DLL, the Direct3D video engine, the installer and updater, and Windows-only settings. Builds are faster, and CI tests Linux only.

## Install

| System | File | Command |
| --- | --- | --- |
| Debian 12+, Ubuntu 22.04+ | `rfiles_…_amd64.deb` / `_arm64.deb` | `sudo apt install ./rfiles_*.deb` |
| Fedora | `rfiles-….x86_64.rpm` / `.aarch64.rpm` | `sudo dnf install ./rfiles-*.rpm` |
| openSUSE | the same `.rpm` | `sudo zypper install ./rfiles-*.rpm` |
| Arch, CachyOS, Manjaro | `rfiles-git-…-x86_64.pkg.tar.zst` | `sudo pacman -U ./rfiles-git-*.pkg.tar.zst` |
| Any distribution | `rfiles-….x86_64.flatpak` / `.aarch64.flatpak` | `flatpak install --user ./rfiles-*.flatpak` |

**Upgrading from rustyfiles 0.6.0-linux.1 or 0.5.2-linux.1:**

- **.deb, .rpm, Arch:** install the rfiles package as above. It replaces the rustyfiles package (pacman asks before removing `rustyfiles-git`).
- **Flatpak:** the app id changed, so rfiles installs alongside the old app. Once rfiles has started once, remove the old app with `flatpak uninstall io.github.benjaminfenech.rustyfiles`. The Flatpak fetches its runtime from Flathub on first install.

Optional extras for previews and drives: poppler (PDFs), libavif / libheif tools (AVIF, HEIC), FFmpeg (older video formats), udisks2 (mounting drives), kio-fuse or gvfs (network locations), git (git status). The [README](https://github.com/benjaminfenech/rfiles#optional-extras) lists the package names for each distribution.

There is no Windows build. For Windows, use [upstream rustyfiles](https://github.com/gelenmedya/rustyfiles/releases).
