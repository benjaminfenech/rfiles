# rfiles

> [!NOTE]
> rfiles is a file manager for Linux, written in Rust. It began as the Linux port of
> [rustyfiles](https://github.com/gelenmedya/rustyfiles) by Özer Güzel, and took its own name
> when it stopped following it: the last upstream release merged was rustyfiles 0.6.0. For
> Windows, use [rustyfiles' releases](https://github.com/gelenmedya/rustyfiles/releases).
> [docs/LINUX_PORT.md](docs/LINUX_PORT.md) keeps the history of the port.

rfiles has tabs, dual panes, archives you open like folders, and **DirectLook**, a
Quick Look–style previewer on <kbd>Space</kbd>. The window never waits on the disk: listing,
icons, thumbnails and every file operation run off the UI thread, so a slow network share or
a folder with 100,000 files can't freeze it. It runs on Wayland and X11.

- [Install](#install)
- [What works](#what-works)
- [Setting up](#setting-up)
- [Skipped: Windows-only](#skipped-windows-only)
- [Pending](#pending)
- [Keyboard](#keyboard)
- [Building and development](#building-and-development)

## Install

Download the package for your system from the
[latest release](https://github.com/benjaminfenech/rfiles/releases/latest):

| System | File | Install with |
| --- | --- | --- |
| Debian 12+, Ubuntu 22.04+ | `rfiles_…_amd64.deb` / `_arm64.deb` | `sudo apt install ./rfiles_*.deb` |
| Fedora | `rfiles-….x86_64.rpm` / `.aarch64.rpm` | `sudo dnf install ./rfiles-*.rpm` |
| openSUSE | the same `.rpm` | `sudo zypper install ./rfiles-*.rpm` |
| Arch, CachyOS, Manjaro | `rfiles-git-…-x86_64.pkg.tar.zst` | `sudo pacman -U ./rfiles-git-*.pkg.tar.zst` |
| Any distribution | `rfiles-….x86_64.flatpak` / `.aarch64.flatpak` | `flatpak install --user ./rfiles-*.flatpak` |

**Updates:** rfiles looks for a newer release soon after it starts and then once a day, and
says so in the status bar and in Settings → About (where the automatic check can be turned off).
Install there takes the same kind of package you installed, checks its SHA-256 against GitHub's,
and installs it: .deb, .rpm and Arch packages through `pkexec` (your desktop asks for your
password), the Flatpak with `flatpak install`. Then Restart runs the new version. A copy built
from source only gets a link to the release.

**Arch from source:** `packaging/arch/PKGBUILD` builds `rfiles-git` from the latest
commit. Run `makepkg -si` there again to update.

```bash
git clone https://github.com/benjaminfenech/rfiles
cd rfiles/packaging/arch
makepkg -si
```

**Flatpak:** installing the bundle fetches the runtime from Flathub. A file manager can't
live in a tight sandbox, so the Flatpak sees your whole filesystem and runs your apps,
terminals and tools on the host. It behaves like the native packages and uses the same
settings.

**Between releases:** every commit to `main` builds the same packages. Download them from
the latest [Linux packages](https://github.com/benjaminfenech/rfiles/actions/workflows/linux-packages.yml)
run (needs a GitHub login).

### Optional extras

Some features use tools you may already have. The .deb and .rpm recommend or suggest them,
and `pacman -Qi rfiles-git` lists them as optional dependencies.

| For | Arch | Debian / Ubuntu | Fedora |
| --- | --- | --- | --- |
| PDF previews | `poppler` | `poppler-utils` | `poppler-utils` |
| AVIF images | `libavif` | `libavif-bin` | `libavif-tools` |
| HEIC images | `libheif` | `libheif-examples` | `libheif-tools` |
| Older video formats (DivX/Xvid, WMV, FLV, Theora) | `ffmpeg` | `ffmpeg` | `ffmpeg-free` |
| Mounting and ejecting drives | `udisks2` | `udisks2` | `udisks2` |
| Network locations (smb://, sftp://, ftp://) | `kio-fuse` or `gvfs-smb` | `kio-fuse` or `gvfs-backends` | `kio-fuse` or `gvfs-smb` |
| Desktop file dialogs, "Open with" | `xdg-desktop-portal` | `xdg-desktop-portal` | `xdg-desktop-portal` |
| Git status | `git` | `git` | `git` |

## What works

- **Browsing:**
  - Tabs, dual pane, and details, grid and column views, with live updates as files change.
  - Grouping, the preview pane, search, colour tags and the Shelf.
  - Breadcrumbs, and `~` in the address bar.
  - Quick search: names, apps and browser bookmarks from Chrome, Chromium, Edge, Brave and Vivaldi.
- **Sidebar:**
  - Your XDG folders.
  - Drives, named by label or mount point. Drives that aren't mounted yet are listed and mount when you open them.
  - Network shares, under Network: every share in `/etc/fstab`, mounted or not, including `x-systemd.automount` ones.
  - Network locations from Dolphin and GTK bookmarks.
- **Files:**
  - Copy, move, rename and delete, with conflict answers, pause, a speed limit and undo/redo.
  - Progress on rfiles' taskbar or dock icon while files copy, on desktops that show it (KDE Plasma, Ubuntu Dock / Dash to Dock, Plank).
  - Bulk rename.
  - The trash, shared with other file managers.
  - Copy and paste with other apps, and dragging files out to them on Wayland.
- **Looks:**
  - Icons from your icon theme.
  - Thumbnails, shared with other file managers. Videos, PDFs and documents go through your installed thumbnailers.
- **Tools:**
  - Folder sizes, remembered across restarts.
  - "Show disk usage", a treemap of what takes the space.
  - Git status, and hashes.
  - "Open with", listing the apps for the type with your default first.
  - "Open in terminal", "Run command here" (or `>` and a command in the address bar), and "Open in editor".
- **Archives:** ZIP, 7z, RAR, the tar family and ISO open like folders, password-protected ones included. Add, delete and rename inside ZIP, 7z and tar, and create ZIP, 7z and tar.zst. Reading is done by [rarchive](https://github.com/gelenmedya/rarchive), rustyfiles' pure-Rust engine.
- **DirectLook:** previews of images (JPEG XL built in, AVIF and HEIC through the tools above), SVG, PDF, video and audio with sound, text and code, archives, and a details card for everything else.
- **Properties window:** General, Details, Hashes, and a drive's page with its space and file system.
- **Desktop integration:**
  - Default file manager, including "Show in folder" from browsers and other apps.
  - Start at login.
  - A tray icon when running in the background.
  - Desktop notifications when long copies finish.
  - The desktop's own file chooser for rfiles' file pickers.
- **Network locations:** `smb://`, `sftp://`, `ftp://`, WebDAV, and `\\server\share` in the address bar, through kio-fuse (KDE) or GVfs (GNOME), with a login prompt when the server asks for one.
- **Languages:** 46 of them, following your `LANG` (or pick one in Settings → General).

## Setting up

**DirectLook on tiling window managers.** DirectLook is its own window, like Quick Look. A
Wayland app can't float or place its own window, so tiling compositors tile it unless a
rule floats it. Its app id is `directlook`; the main window's is `rfiles`
(`io.github.benjaminfenech.rfiles` in the Flatpak).

```lua
-- Hyprland, Lua config (hyprland.lua)
hl.window_rule({ match = { class = "^(directlook)$" }, float = true, center = true })
```

```bash
# Sway (~/.config/sway/config)
for_window [app_id="directlook"] floating enable
# i3 (X11)
for_window [class="directlook"] floating enable
```

For Hyprland's classic `hyprland.conf`, add a rule that floats (and centers) class
`directlook`. The syntax depends on your Hyprland version
([window rules](https://wiki.hypr.land/Configuring/Basics/Window-Rules/)).

**Quick search shortcut.** Turn on Settings → Quick search → "Global shortcut": rfiles
asks your desktop for a shortcut through the desktop portal. KDE Plasma and GNOME 48+ ask you
to confirm the key (Alt+Space is suggested); Hyprland binds it in its config, under the name
`hyprctl globalshortcuts` lists for it:

```bash
bind = ALT, SPACE, global, rfiles:quick-search
```

It works while rfiles runs (Settings → Advanced → run in the background keeps it running with
no window open). Without the portal, bind a key to `rfiles --quick-search` instead (Flatpak:
`flatpak run io.github.benjaminfenech.rfiles --quick-search`). On Wayland the quick search
opens as an overlay, so tiling compositors don't tile it.

**Searching inside files.** Start a search with `content:` (`content:quarterly budget`), or
press "Search inside files" when a name search finds nothing. rfiles asks your desktop's file
index when it is running and covers the folder: Baloo on KDE Plasma, Tracker (LocalSearch) on
GNOME. Both read PDFs and office documents too. Elsewhere rfiles reads the files itself: text
files up to 64 MB, slower on big folders.

**Terminal.** "Open in terminal" (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>`</kbd>) opens your
terminal in the current folder. To run a command there, type `>` and the command in the address
bar (`> make`), or use "Run command here…" from the folder's right-click menu or the command
palette. The terminal stays open with a shell afterwards. Settings → Advanced → Terminal picks
the terminal. Automatic uses `$TERMINAL`, `xdg-terminals.list`, KDE's or GNOME's setting,
then the first common terminal installed.

**Default file manager.** Settings → Advanced → "Set as default file manager" makes folders,
and "Show in folder" in browsers, open in rfiles, for your user only. Turning it off
brings your previous file manager back.

**Drives.** The eject button on a USB stick, SD card or disc in the sidebar removes it safely;
right-click a drive to unmount it. Encrypted (LUKS) drives show with a lock and ask for
their passphrase when opened. fstab network shares whose server can't be reached are marked
offline. Mounting an internal disk may ask for an administrator password, which needs a
polkit agent. KDE and GNOME start one; on Hyprland or Sway, start e.g. `hyprpolkitagent` or
`polkit-kde-agent`. Settings → Files and folders → "Show drives as" picks how drives are
named.

**Network folders.** The address bar shows them by their address (`smb://server/share/…`).
Settings → Files and folders → "Show network folders by their address" switches to where
they are mounted (`/run/user/…`). Pinning a network folder pins its address.

**Tray.** Settings → Advanced → "Keep running in the background" leaves an icon in the tray
when the last window closes. KDE, Waybar, Noctalia and most bars show it; GNOME needs the
AppIndicator extension.

**Copy and paste with other apps** works on X11 and on Wayland compositors with data-control
(KDE, Hyprland, Sway). On GNOME's Wayland session it works inside rfiles only.

## Skipped: Windows-only

These come from rustyfiles' Windows app and have no Linux counterpart, so rfiles doesn't
have them:

- **Explorer integration:** DirectLook over Explorer and the desktop, the Explorer context menu and its shell extensions, `shell:` paths, Win+E.
- **Windows shell features:** jump lists, the taskbar preview and taskbar progress, Libraries, Windows Search, OneDrive status, devices under This PC, preview handlers, rfiles as other apps' Open/Save dialog, `.lnk` shortcuts, the global hotkey, and the speed test against File Explorer.
- **Properties tabs:** Shortcut, Compatibility, Security (Windows ACLs), Digital signatures (Authenticode), Sharing, Customize (desktop.ini folder icons), Disk Cleanup, "Unblock", and the archive, indexing, compression and encryption attributes. Hidden is a dot name on Linux, and read-only is the write permission.
- **Prompts:** upstream's strips asking for a GitHub star or a donation are gone. Settings → About still has the links.
- **Platform plumbing:** Velopack install and auto-update (rfiles has its own updater for the Linux packages), Mica, the hybrid GPU/CPU renderer and its popup-window menus, protected system files, and upstream's Direct3D video engine (rustdecoder). Video plays through rf-player and FFmpeg.

## Pending

Not done yet, roughly in order of how much they'd help:

- **DirectLook outside rfiles.** Space over other apps needs a global hotkey, which Wayland doesn't give apps. rfiles now has portal global shortcuts (quick search uses one), but other apps don't say which file is selected, so DirectLook has nothing to show over them.
- **rfiles as other apps' Open/Save dialog.** On Linux that's an xdg-desktop-portal FileChooser backend, a project of its own.
- **DirectLook details:** RAW photos beyond the embedded JPEG, version strings of Windows programs, and certificate details.
- **Distribution:** an AUR package (waiting for AUR account registration to reopen), an apt/dnf repository (COPR, OBS), and Flathub. Until then, install from [Releases](https://github.com/benjaminfenech/rfiles/releases); the built-in updater handles later versions.
- **Hand checks:** a few features are built and unit-tested but haven't been tried by hand on every desktop. [docs/LINUX_PORT.md](docs/LINUX_PORT.md) says which.

## Keyboard

| Keys | Action |
|---|---|
| <kbd>Ctrl</kbd>+<kbd>T</kbd> / <kbd>Ctrl</kbd>+<kbd>W</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | New tab, close tab, reopen closed tab |
| <kbd>Ctrl</kbd>+<kbd>L</kbd> or <kbd>Alt</kbd>+<kbd>D</kbd> | Address bar |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Search (<kbd>Enter</kbd> searches subfolders too) |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> or <kbd>F1</kbd> | Command palette |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>1</kbd> / <kbd>2</kbd> / <kbd>3</kbd> | Details, grid, columns |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd>, <kbd>F6</kbd> | Dual pane, switch pane |
| <kbd>Alt</kbd>+<kbd>P</kbd>, <kbd>Ctrl</kbd>+<kbd>B</kbd> | Preview pane, sidebar |
| <kbd>Space</kbd> | DirectLook |
| <kbd>F2</kbd> | Rename (bulk rename with several items selected) |
| <kbd>Ctrl</kbd>+<kbd>Z</kbd> / <kbd>Ctrl</kbd>+<kbd>Y</kbd> | Undo, redo |
| <kbd>Ctrl</kbd>+<kbd>H</kbd> | Show hidden items |

Every shortcut can be changed in Settings → Keyboard shortcuts: click a command, then press
the new keys.

## Building and development

You need [Rust](https://rustup.rs) 1.95 or newer, a C compiler, and these libraries:

```bash
# Arch / CachyOS / Manjaro
sudo pacman -S --needed base-devel libxkbcommon-x11 wayland fontconfig freetype2 alsa-lib

# Debian / Ubuntu (what CI uses)
sudo apt install build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libwayland-dev libfontconfig1-dev libfreetype-dev libasound2-dev
```

```bash
git clone https://github.com/benjaminfenech/rfiles
cd rfiles
cargo run --release -p rf-app --bin rfiles
```

**Packages:** `packaging/linux/build-packages.sh` makes the .deb and .rpm from a release
build (needs [nfpm](https://nfpm.goreleaser.com)). The Flatpak builds with:

```bash
flatpak-builder --user --install --force-clean build-flatpak packaging/flatpak/io.github.benjaminfenech.rfiles.yml
```

**Releases:** set the version in `Cargo.toml`, put its tag in `packaging/linux/RELEASE`
(`v<version>`, like `v0.7.0`) and the notes in `packaging/linux/release-notes.md`, in a PR.
When it lands on `main`, CI tags it, builds every package and publishes the release.

**rustyfiles:** rfiles no longer merges it. A cross-platform fix from there can still be
cherry-picked (`git remote add upstream https://github.com/gelenmedya/rustyfiles`);
[docs/LINUX_PORT.md](docs/LINUX_PORT.md) logs the merges up to 0.6.0.

**Settings** live in `~/.config/rfiles/settings.toml`, DirectLook's in `directlook.toml`
next to it.

| Variable | Effect |
|---|---|
| `RFILES_CONFIG_DIR` = folder | Keep settings in another folder (portable use, tests) |
| `RFILES_MULTI_INSTANCE=1` | Allow several copies at once (tests) |
| `RFILES_LOOK_INSTANCE` = name | A separate DirectLook instance (tests) |
| `RFILES_FRAME_LOG=1` | Log frame times |
| `RFILES_LOOK_TRACE=1` | Log DirectLook's timings to stderr |
| `RFILES_MUTE=1` | DirectLook plays without sound (tests) |

| Crate | What it holds |
|---|---|
| `crates/rf-app` | The interface, and the two programs: `rfiles` and `directlook` (`src/look/`) |
| `crates/rf-linux` | The Linux platform layer: files, drives, trash, icons, thumbnails, the desktop |
| `crates/rf-core` | The model: locations, history, entries, formatting |
| `crates/rf-archive` | Archive operations: browse, extract, create, edit |
| `crates/rf-player` | DirectLook's video and audio player |

The interface is built with [GPUI](https://gpui.rs), the framework behind the Zed editor.

Bug reports and pull requests are welcome.

## Thanks

To Özer Güzel and everyone behind [rustyfiles](https://github.com/gelenmedya/rustyfiles),
which rfiles grew out of. rustyfiles credits [Files](https://github.com/files-community/Files),
[GPUI](https://gpui.rs) and [Zed](https://github.com/zed-industries/zed),
[gpui-kit](https://github.com/longbridge/gpui-kit), [QuickLook](https://github.com/QL-Win/QuickLook),
UnRAR's source, and many crates. On Linux, rfiles also leans on
[ksni](https://github.com/iovxw/ksni), [ashpd](https://github.com/bilelmoussaoui/ashpd),
[zbus](https://github.com/dbus2/zbus), [notify](https://github.com/notify-rs/notify),
[jxl-oxide](https://github.com/tirr-c/jxl-oxide), poppler, libavif, libheif, FFmpeg, UDisks2,
kio-fuse and GVfs.

## License

MIT, see [LICENSE](LICENSE). It keeps rustyfiles' copyright notice next to rfiles' own.
