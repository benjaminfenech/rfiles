The first release of the **Linux port of [rustyfiles](https://github.com/gelenmedya/rustyfiles)**, built from upstream 0.5.2. It runs on Wayland and X11.

## Install

| System | File | Command |
| --- | --- | --- |
| Debian 12+, Ubuntu 22.04+ | `rustyfiles_…_amd64.deb` / `_arm64.deb` | `sudo apt install ./rustyfiles_*.deb` |
| Fedora | `rustyfiles-….x86_64.rpm` / `.aarch64.rpm` | `sudo dnf install ./rustyfiles-*.rpm` |
| openSUSE | the same `.rpm` | `sudo zypper install ./rustyfiles-*.rpm` |
| Arch, CachyOS, Manjaro | `rustyfiles-git-…-x86_64.pkg.tar.zst` | `sudo pacman -U ./rustyfiles-git-*.pkg.tar.zst` |
| Any distribution | `rustyfiles-….x86_64.flatpak` / `.aarch64.flatpak` | `flatpak install --user ./rustyfiles-*.flatpak` |

The Flatpak fetches its runtime from Flathub on install. It can see your whole filesystem and runs your apps and tools on the host, so it behaves like the native packages and uses the same settings.

Optional extras for previews and drives: poppler (PDFs), libavif / libheif tools (AVIF, HEIC), FFmpeg (older video formats), udisks2 (mounting drives), kio-fuse or gvfs (network locations), git (git status). The .deb and .rpm recommend or suggest them.

## What works

- **Browsing:** tabs, dual panes, the column view, live updates, breadcrumbs and `~` in the address bar, and the sidebar with your XDG folders and drives. Drives that aren't mounted are listed too and mount through UDisks2 when you open them; unmount and eject are there as well.
- **File operations:** copy, move, rename and delete, with conflict answers, pause, a speed limit and undo. The trash is shared with other file managers. Copy and paste works with other apps on X11 and on Wayland compositors with data-control (KDE, Hyprland, Sway).
- **Look and feel:** icons from your icon theme, and thumbnails shared with other file managers.
- **Tools:** folder sizes, disk usage, git status, hashes, "Open with" (your default app first), "Open terminal here", and quick search (bind `rustyfiles --quick-search` to a key in your desktop's settings).
- **DirectLook (Space):** previews of images (JPEG XL built in), SVG, PDF, video and audio with sound, text and archives.
- **Default file manager:** rustyfiles can be made the default (Settings), which includes answering "Show in folder" from other apps.

Not yet on Linux: the Open/Save dialogs inside other apps, and Windows-only features (jump lists, Windows Search, Libraries, the taskbar preview, auto-update). See [docs/LINUX_PORT.md](https://github.com/benjaminfenech/rustyfiles-linux/blob/main/docs/LINUX_PORT.md) for the details.

This release has no Windows build: for Windows, use [upstream's releases](https://github.com/gelenmedya/rustyfiles/releases).
