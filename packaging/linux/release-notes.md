Permissions, links, templates, phones and cameras, network servers, folder sharing and a folder tree: the features other Linux file managers have that rfiles didn't.

## What's new since 0.10.0

- **Permissions in Properties.** Change the owner group and the read, write and run permissions for owner, group and others, or tick "Allow running as a program". It works on several items at once, and "Apply to everything inside" goes through a folder's contents (files keep their run permission as it was).
- **Owner, group and permissions columns** in the details view, sortable like the others.
- **Links.** "Create link" in the right-click menu, "Paste as link", and Ctrl+Shift while dropping. Links get a small arrow badge, and undo removes them.
- **New file from a template.** Files and folders in `~/Templates` show up in the New menu.
- **Select by pattern** (Ctrl+S): type `*.jpg` or `report*` to select the matching items.
- **Paste an image or text as a file.** With a copied image or text on the clipboard, Paste writes "Pasted image.png" or "Pasted text.txt".
- **Mount a disk image.** Right-click an ISO or IMG and choose Mount; it shows up as a drive (opening it as an archive still works too).
- **Search filters:** `size:>10M`, `modified:7d`, `created:2024`, `type:image,video`, alone or with a name.
- **Your actions in the right-click menu.** Dolphin service menus and Nautilus scripts already on your system appear under Actions, matched to the file type. Settings → General → Your actions adds and edits your own; they're saved as Dolphin service menus, so Dolphin shows them too.
- **Folder tree.** Settings → Sidebar sections → Folder tree gives pinned folders, drives and Home an arrow that opens them into their subfolders.
- **Phones, tablets and cameras** plugged in over USB show under Drives and open as folders. This needs gvfs-mtp (phones), gvfs-gphoto2 (cameras) or gvfs-afc (iPhone, iPad); without gvfs, `jmtpfs`, `gphotofs` or `ifuse` work too.
- **File servers on your network** that announce themselves (Samba with Avahi, most NAS boxes, Macs) show under Network: SMB, SFTP, WebDAV, FTP and NFS. Needs Avahi.
- **Share a folder on the network** from its Properties → Sharing: a share name, read-only or writable, guests or not, and a Copy button for its `smb://` address. Uses Samba's user shares, so Samba needs to be installed with user shares turned on.
- **Open in Disks.** Right-click a drive to open it in GNOME Disks, KDE Partition Manager or GParted, whichever is installed, to format or partition it.
- **Overview and Home.** The Windows-style start page is now called Overview, and a new Home entry in the sidebar opens your home folder. Alt+Home goes to your home folder, as in Dolphin and Nautilus.
- Fixed flaky tests in the file watcher, journal and copy-resume code.

**Updating:** from 0.8.0 or newer, Settings → About → Install (or the status bar's "rfiles 0.11.0 available"). From 0.7.0 or older, install by hand once as below.

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
