Error messages and other texts that showed in Turkish now follow your language.

## What's new since 0.8.1

- **No more stray Turkish.** Errors from archives, git, drives, renaming and file operations, and a few other texts, showed in Turkish whatever language you picked. They now show in your language, or in English where there is no translation yet.
- **Turkish is now an ordinary translation.** English is the language rfiles is written in; Turkish users keep a complete Turkish interface.
- **Some texts that were untranslated in every language** (left behind by the rename to rfiles) are translated again.

**Updating:** from 0.8.0 or newer, Settings → About → Install (or the status bar's "rfiles 0.8.2 available"). From 0.7.0 or older, install by hand once as below.

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
