Security fixes, signed updates, and the wallpaper and font actions now work.

## What's new since 0.9.0

- **Browsing a git repository can no longer run commands from it.** rfiles reads git status and history by itself when you open a folder. A repository's own settings could make that run a program (an fsmonitor, a filter, a signature checker), so a downloaded or extracted folder could run code just by being opened. rfiles now switches those off for its own reads; your commits, pulls and pushes work as before, hooks and Git LFS included.
- **Files opened from archives stay private.** They were extracted into a shared folder in `/tmp` that other users on the computer could read, including files from password-protected archives. They now go into `~/.cache/rfiles/tmp`, readable only by you, and the old folder is removed. Thumbnails and other temporary files moved there too.
- **Signed updates.** Releases now come with a signed list of checksums, and the updater installs a package only if that signature matches the key built into rfiles. This applies to updates after this one: 0.9.0 installs 0.9.1 the old way.
- **Set as desktop background** works on desktops that support it (KDE Plasma, GNOME) and is hidden elsewhere. **Install font** copies the font to `~/.local/share/fonts`.
- **"Run as administrator"**, a leftover from Windows, is gone from the menu.
- **Deleting a ZIP (or other archive) file works.** It failed with "Nothing to delete", taking the file for an item inside the archive. (A fix from rustyfiles 0.6.5.)
- **Long names in the grid view end in "…"** instead of being cut at both ends of the tile. (A fix from rustyfiles 0.7.0.)
- The git commit column tells apart files whose names differ only in case (`Makefile` and `makefile`).
- Waiting for helper programs (thumbnailers, the update installer) no longer wakes the computer up every few milliseconds.

**Updating:** from 0.8.0 or newer, Settings → About → Install (or the status bar's "rfiles 0.9.1 available"). From 0.7.0 or older, install by hand once as below.

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
