Two changes you'll notice: you can type your own date format, and rfiles can now update itself.

## What's new since 0.7.0

- **Updates from inside rfiles.** rfiles checks GitHub for a newer release soon after it starts and then once a day. When there is one, the status bar says so, and Settings → About has an Install button. It downloads the same kind of package you installed (.deb, .rpm, Arch or Flatpak), checks its SHA-256 against the one GitHub lists, and installs it: through `pkexec` for system packages (your desktop asks for your password), or `flatpak install` for the Flatpak. Then Restart runs the new version. The automatic check can be turned off in Settings → About.
- **Type your own date format.** Settings → Date format is now a text field, such as `dd.mm.yyyy hh:mm` or `yyyy-mm-dd`, with a live preview and a list of the accepted parts (`dd`, `mm`, `mmm` for "Nov", `yyyy`, `hh:mm:ss` and separators such as `. / - :`; upper or lower case). A separate switch shows recent dates as "5 minutes ago". Your old choice carries over.
- **Build dates follow the setting.** Settings → About showed its build date as year-month-day whatever you chose; it now uses your format.

**Coming from 0.7.0:** 0.7.0 has no updater, so install 0.8.0 by hand once (below). After that, rfiles updates itself.

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
