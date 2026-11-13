Search inside files, run commands in a terminal, and use rfiles as the Open/Save dialog of other apps.

## What's new since 0.8.2

- **Search inside files.** Start a search with `content:` (for example `content: invoice`), or press "Search inside files" when a name search finds nothing. rfiles asks Baloo (KDE) or LocalSearch/Tracker (GNOME) when they index the folder, and reads the files itself otherwise.
- **Terminal integration.** "Run command here…" in the folder's menu, or type `>` followed by a command in the address bar, and it runs in that folder in your terminal. Settings → Advanced → Terminal picks which terminal; Automatic follows your desktop's default.
- **rfiles as other apps' Open/Save dialog.** Turn on "Use rfiles for Open and Save dialogs" in Settings → Advanced (deb, rpm and Arch packages; not the Flatpak). On tiling compositors, add a floating rule for the app id `rfiles-dialog`.
- **Camera RAW photos at full quality** in DirectLook when LibRaw (`dcraw_emu`) or dcraw is installed.
- **Certificate details** (subject, issuer, validity) and **Windows program versions** (.exe, .dll) in DirectLook and Properties.
- **Every text translated in every language.** These new translations are machine-made; corrections from native speakers are welcome.

**Updating:** from 0.8.0 or newer, Settings → About → Install (or the status bar's "rfiles 0.9.0 available"). From 0.7.0 or older, install by hand once as below.

## Install

| System | File | Command |
| --- | --- | --- |
| Debian 12+, Ubuntu 22.04+ | `rfiles_…_amd64.deb` / `_arm64.deb` | `sudo apt install ./rfiles_*.deb` |
| Fedora | `rfiles-….x86_64.rpm` / `.aarch64.rpm` | `sudo dnf install ./rfiles-*.rpm` |
| openSUSE | the same `.rpm` | `sudo zypper install ./rfiles-*.rpm` |
| Arch, CachyOS, Manjaro | `rfiles-git-…-x86_64.pkg.tar.zst` | `sudo pacman -U ./rfiles-git-*.pkg.tar.zst` |
| Any distribution | `rfiles-….x86_64.flatpak` / `.aarch64.flatpak` | `flatpak install --user ./rfiles-*.flatpak` |

Optional extras for previews and drives: poppler (PDFs), libavif / libheif tools (AVIF, HEIC), LibRaw (camera RAW), FFmpeg (older video formats), udisks2 (mounting drives), kio-fuse or gvfs (network locations), git (git status). The [README](https://github.com/benjaminfenech/rfiles#optional-extras) lists the package names for each distribution.

There is no Windows build. For Windows, use [upstream rustyfiles](https://github.com/gelenmedya/rustyfiles/releases).
