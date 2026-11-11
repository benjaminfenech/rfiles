# Porting rustyfiles to Linux

Status: **Phase 0 done** (2026-11-08): the workspace builds and its tests pass on Linux, and
`cargo run -p rf-app --bin rustyfiles` opens the window and browses folders. Phases 1–3 are
still to do. Plan written against `f28d4ac`.

## Where things stand

On Linux (Rust 1.99, Arch) `cargo build --workspace` fails at the `windows` crate. Only
`rf-core` builds.

| Crate | Lines | Windows use | What it needs |
|---|--:|---|---|
| rf-core | 2.0k | none | builds as is |
| rf-archive | 5.3k | 4 files: code page lookup, FILETIME conversion, WinRAR registry lookup, `MetadataExt` | small: about a day |
| rf-player | 1.7k | D3D11 presenter, WASAPI audio | new presenter and audio output |
| rf-dialog | 1.0k | an Open/Save dialog DLL that gets injected into other apps | leave out on Linux |
| rf-win | 11.7k | 50 of 57 files: the platform layer | a Linux counterpart, `rf-linux` |
| rf-app | 26.5k | 16 files call Windows directly; 44 `rf_win` modules used | cfg gates plus a platform alias |

The UI layer is the easy part. `gpui-kit` already pulls in `gpui-pre-linux` (Wayland and
X11), and it compiled on this machine. Only the forked Windows renderer (`gpui_windows::hybrid`,
`SMOOTH_SCROLL`, `KEEP_ACTIVE_LOOK`, `dnd_hint`) has no Linux equivalent.

## Approach

**Keep rf-win as it is and add a sibling crate, `rf-linux`, with the same module names and
function signatures.** rf-app chooses one at compile time:

```rust
// crates/rf-app/src/lib.rs
#[cfg(windows)]   pub use rf_win as platform;
#[cfg(unix)]      pub use rf_linux as platform;
```

Then `rf_win::` becomes `platform::` throughout rf-app (a mechanical rename).

*As built:* instead of the rename, `rf-app/src/lib.rs` has `#[cfg(unix)] extern crate rf_linux
as rf_win;`, so rf-app's code didn't change at all. The same trick covers the GPUI fork:
`crates/gpui-windows-shim` builds a no-op crate named `gpui_windows` on Linux.

Why not a trait-based abstraction: rf-app calls about 45 modules made of free functions,
often with Windows-shaped types (`HWND` owners on the shell thread, `Transfer`, `Drive`).
Mirroring the API keeps rf-app's diff small and makes upstream changes from
`gelenmedya/rustyfiles` easy to merge. Shared types (`Drive`, `ListError`, `Transfer`, and
so on) move into rf-core so both backends return the same thing.

Rules that keep upstream merges cheap:
- Windows code stays where it is; Linux code goes in new files or `#[cfg(unix)]` blocks.
- Windows-only dependencies move to `[target.'cfg(windows)'.dependencies]`.
- Features Linux lacks get a stub that returns `None`/`Err(Unsupported)`, and the UI hides
  their entry. No dead buttons.

## Phases

### Phase 0: it compiles and opens a window (about 1 week)

1. Cargo: gate `windows`, `windows-core`, `windows-registry`, `velopack`, `embed-resource`,
   `gpui-pre-windows` and the `[patch]` entry to `cfg(windows)`. Exclude `rf-dialog` and
   `tools/gpustress` from the workspace on Linux, or cfg the whole crate.
2. rf-archive: `names.rs` uses `encoding_rs` with a locale-derived code page (default CP437
   for zip); `timeconv.rs` uses chrono; `winrar.rs` looks for `rar`/`unrar` on `PATH`;
   `lib.rs` uses `std::os::unix::fs::MetadataExt`.
3. rf-player: put the D3D presenter and WASAPI behind cfg; on Linux, start with an
   "unsupported" stub.
4. Create `rf-linux` with every module rf-app uses, all stubs, and `todo!()` nowhere.
5. rf-app: add the `platform` alias; put `gpui_windows::*`, `installer.rs` (Velopack), the
   Explorer forwarding in `main()` and the `/select,` / jump list argument handling behind cfg.
   Add a small `renderer` shim so the settings and status code that read `hybrid::mode()`
   compile on both.
6. Fonts: "Segoe UI Variable Text"/"Segoe UI" → system UI font (from the portal or fontconfig;
   fallback "Cantarell"/"Noto Sans"); "Cascadia Mono/Code" → system monospace font.

Done when: `cargo build --workspace` passes on Linux, `cargo run` opens an empty window, and
Windows still builds (CI job on `windows-latest`).

#### Phase 0 as built

- `rf-linux` started as a generated copy of rf-win's public API (types kept, Windows types
  replaced by `rf_linux::Opaque`/`RECT`, portable function bodies kept, the rest returning
  `None`/`false`/empty/"not implemented on Linux yet" errors). No function panics.
- Already real on Linux: `listing`, `drives` (`/proc/self/mountinfo` + `statvfs`), `known`
  (XDG user dirs), `watch` (inotify), `sta` (one worker thread), `single_instance` (Unix
  socket in `$XDG_RUNTIME_DIR`), `foldersize`, `thumbs` (image files only, no type icons
  yet), `shell::open` (`xdg-open`), `shell::open_terminal`, `fileinfo`, `hash`, `git`.
- rf-app's direct Win32 calls go through `rf-app/src/sys.rs` (cursor position, Shift/Ctrl,
  double-click time, file attributes, UI language, fonts). On Linux the modifier keys read as
  "not held" and the cursor as (0, 0) until Phase 1 reads them from GPUI; only the Windows
  shell menus used them.
- `rf-core::clean_path` has a Unix version (it used to turn `/` into `\`), and the
  breadcrumb bar gets a `/` crumb. The address bar expands `~`.
- Velopack is off on Linux (the updater reports "not installed").
- CI: a `linux` job runs the tests and builds rf-app on Ubuntu.
- Not checked yet: the Windows build after these changes. This machine has no Windows
  target installed; the existing `windows-latest` CI job checks it.

### Phase 1: a usable file manager (about 3–4 weeks)

Progress (2026-11-08): **file operations done** (copy, move, rename, delete, the Trash;
copy/paste inside rustyfiles), **icons and thumbnails done**, **menus drawn in-window**,
**"Open with" done**, **Shift/Ctrl and the path audit done**, **clipboard shared with other
apps**. Phase 1 is complete; Phase 2 (DirectLook and media) is next.


| Module | Linux implementation |
|---|---|
| listing, search ✅, foldersize ✅, nameindex ✅, diskusage ✅ | `std::fs::read_dir` + `std::os::unix` metadata; hidden = leading dot; skip symlink loops (track dev+inode) instead of junctions |
| known, localized, libraries | `dirs` + `xdg-user-dirs`; drop localized names and libraries |
| drives, volume | `/proc/self/mountinfo` + `statvfs`; filter pseudo filesystems |
| watch | `notify` already uses inotify; just ungate |
| thumbs | ✅ `xdg_icons`: shared-mime-info (by extension) → the desktop's icon theme per the Icon Theme spec (theme from kdeglobals / GTK settings / gsettings, inherited themes, hicolor, pixmaps), SVGs drawn with resvg. `xdg_thumbs`: the shared `~/.cache/thumbnails` (read and written, `Thumb::MTime` checked, failures in `fail/rustyfiles`), images and SVGs decoded here, other types through the installed `.thumbnailer`s with a 15 s limit. `cargo run -p rf-linux --example icon_lookup` / `--example thumbnail` to check a system |
| shell::open, open_with, open_terminal | ✅ `xdg-open`; `$TERMINAL` or the first common terminal. "Open with": `xdg_apps` reads the `.desktop` files and `mimeapps.list` (defaults, added / removed associations, parent types via `subclasses`) and launches per the Desktop Entry spec (`%f %F %u %U %i %c %k`, `Terminal=true`); the context menu's "Open with" lists them (default first), the standalone "Open with…" asks the desktop portal's app chooser. `cargo run -p rf-linux --example open_with -- <file>` shows what a system offers |
| fastcopy, fastdelete, fileops | ✅ rf-win's engine and tests, ported: `copy_file_range` in 8 MB chunks (read/write fallback), modes and times kept, `renameat2(RENAME_NOREPLACE)`, same device = rename. `fileops` is the fallback engine (symlinks copied as links). Extended attributes and ACLs are not copied yet. Crash-safe job journal and resume (upstream 0.3.2) ported: rf-win's `journal` format with `flock` in place of share modes (a locked journal is a running job) and escaped paths (Linux names can hold tabs, line breaks and non-UTF-8 bytes); big files (64 MB+) resume mid-file from checkpoints (the copy `fdatasync`ed and its length journaled every 2 s, the source's mtime checked on resume) where Windows uses `COPY_FILE_RESTARTABLE`. `examples/crashcopy.rs` cuts a copy off for testing (optional MB/s limit) |
| recycle | ✅ freedesktop Trash spec, our own: home trash + per-disk `.Trash/$uid` / `.Trash-$uid`, list / restore / empty, undo of a delete. The UI still says "Recycle Bin" (renaming needs new translations) |
| clipboard | ✅ `x-special/gnome-copied-files`, `text/uri-list` + `application/x-kde-cutselection`, and plain paths. Wayland through data-control (`wl-clipboard-rs`: wlroots compositors, KDE), X11 as the CLIPBOARD owner (`x11rb`); GNOME's Wayland has no data-control, so there copy/paste stays inside rustyfiles. `cargo run -p rf-linux --example clipboard` checks a system |
| git, hash, encoding, complete | mostly portable: remove `creation_flags(CREATE_NO_WINDOW)` and Windows-path handling |
| single_instance | Unix socket in `$XDG_RUNTIME_DIR` (or a D-Bus name) |
| accent, sysparams, window::set_dark_mode | xdg-desktop-portal Settings (`color-scheme`, `accent-color`) via `ashpd`; GPUI window appearance |
| shortcut | `.desktop` files and symlinks instead of `.lnk` |
| props, fileinfo | `statx` for birth time; media properties from the existing decoders (drop the Windows property system) |
| ctxmenu | no native shell menu exists on Linux: use the app's own menu (`fluent_menu`/`ctx_menu`) with Open with, Terminal, Trash, Properties, plus custom actions later |
| sta | not needed: a plain worker thread pool; keep `run()` so call sites compile |

Names that aren't valid UTF-8 are shown lossily by `listing` today; keep the raw bytes so they
still open and rename.

rf-app path audit ✅ (2026-11-08): DirectLook no longer turns `/` into `\\`; the address bar
and folder aliases use the platform separator; the command palette takes `/…` and `~/…`;
rename and archive names follow Linux's rule (only `/` is forbidden); a case-only rename is a
clash on Linux; long paths wrap after `/`; a plain drag compares devices (it used to move
everything on Linux, every path starting at `/`). Shift/Ctrl come from GPUI's events
(`sys::note_modifiers`), and the extension-change question and rename errors use the app's
own dialog (Linux has no shell message box). Left as is because harmless on Linux:
drive-letter checks that never match, archive-path normalization, per-folder layout keys
compared case-insensitively.

Done when: you can browse, copy, move, rename, trash, search and open files on Wayland and
X11 without freezes, including on a slow NFS/SMB mount (the "never waits on the disk" promise
still holds).

### Phase 2: DirectLook and media (about 2–3 weeks)

Progress (2026-11-08): **DirectLook window done**: it stays its own window (decided with the
user) with app id `directlook` for window rules (README has Hyprland / Sway / i3 rules),
sized from GPUI's display info and resized with `window.resize` (`sys::work_area_*`,
`sys::set_window_rect`). Hyprland ignores a floating window's own resize, so on Linux a
clearly different size (more than 12 % and 48 px on a side) reopens DirectLook at that size
(`sys::reopen_for_size`, `LookManager::reopen`: the new window opens first, so rustyfiles
keeps following the selection). Image sizes and SVG done.

Progress (2026-11-09): **PDF preview done** through poppler's command-line tools, no library
to link: `pdfinfo -box` reads every page's crop box and rotation once (575 pages in about
40 ms), and `pdftoppm -scale-to` renders one page to PPM on stdout as it scrolls into view
(`-scale-to-x/-y` would apply before the page's rotation). A missing poppler shows an
install hint.

Progress (2026-11-09): **full-size JXL, AVIF and HEIC done** (they were thumbnails capped at
1024 px). JPEG XL decodes in-process with `jxl-oxide` (pure Rust). AVIF and HEIC go through
libavif's `avifdec` or libheif's `heif-dec` / `heif-convert` into a PNG in a private temp
folder (both apply the container's rotation), so building needs no codec library; about
0.1 s for 3000×2000. `look/heif.rs` reads the primary image's size (`ispe` + `irot`) from
the header so the window opens at the right shape, and the Exif item (`iinf` + `iloc`) for
the info card. With no decoder installed, DirectLook says what to install (the image viewer
now drops its early icon placeholder when decoding fails, so the message shows). RAW still
shows its embedded JPEG.

Progress (2026-11-09): **sound in video and audio previews done** with `cpal` (ALSA; PipeWire
and PulseAudio serve it through their ALSA plugins, so it shows up as "PipeWire ALSA
[directlook]"). `rf-player`'s `wasapi_sink()` keeps its name on Linux and returns the cpal
sink: a 100 ms queue between the player and cpal's callback, and a clock that counts the
frames the callback took minus what the device still holds (cpal's predicted playback
time), moving smoothly between callbacks, frozen while paused. The stream lives on its own
thread (cpal streams can't move between threads) and ends when DirectLook leaves the file.
Building needs ALSA's headers (`alsa-lib` / `libasound2-dev`).

Progress (2026-11-09): **FFmpeg fallback done** for what FilmCraft can't play (it covers
H.264/HEVC/AV1/VP8/VP9/MPEG-2/MJPEG and the usual audio already). `rf-player/src/ffmpeg.rs`
implements FilmCraft's `MediaSource`, so the player and the app are unchanged: `ffprobe`
describes the file, one `ffmpeg` streams 8-bit 4:2:0 frames at a constant rate, another
streams f32 audio at the player's rate; both read through jumps under 2 s and start again
(`-ss`) for seeks. Extensions FilmCraft doesn't list (WMV/WMA, FLV, Theora, AC-3, …) go
straight to FFmpeg; a file FilmCraft opens but can't decode (DivX/Xvid in AVI) switches on
the first "unsupported" error. A 720p DivX file plays in real time. Linux's
`preview_host::is_playable` now says no, so neither DirectLook nor the preview pane offers
the Media Foundation player that doesn't exist here. Next: Phase 3, or PE version strings and
certificates for the info card.

- **Global Space key over other apps does not work on Wayland** (no global keyboard hooks,
  no foreground-window inspection). On Linux DirectLook works inside rustyfiles only;
  `look/hook.rs` and `look/target.rs` stay Windows-only. Optional later: an X11-only hook
  or a desktop-portal GlobalShortcuts binding.
- ~~`look/decode.rs`~~ JXL via `jxl-oxide`, AVIF/HEIC via `avifdec` / `heif-dec` (optional
  runtime dependencies). RAW: still the embedded JPEG; `rawler` later if that's not enough.
- ~~`look/pdf.rs`~~ done with poppler's `pdfinfo` / `pdftoppm` (runtime dependency for packaging).
  Password-protected PDFs show a message, as on Windows.
- `preview_host` (Windows preview handlers): no equivalent; leave out.
- DirectLook info card (from upstream, 2026-11-08): `pe` shows only the architecture on Linux;
  read the version strings from the `VS_VERSIONINFO` resource ourselves. `cert` returns
  nothing on Linux; decode with an X.509 crate (`x509-parser`).
- rf-player: present decoded frames as a GPUI image texture (simplest), and later a
  wgpu/Vulkan presenter if it is too slow. ~~Audio via `cpal`~~ done (ALSA backend).

### Phase 3: desktop integration (about 2 weeks, can be split)

- ~~Default file manager~~ **done** (2026-11-09): Settings → Advanced → "Set as default file
  manager" (`rf-linux` `default_fm`) writes `rustyfiles.desktop` (`Exec=… %F`), makes it the
  `inode/directory` default in the user's `mimeapps.list` (and in a desktop-specific one that
  already decides folders, since that wins), keeping the old default under `[X-rustyfiles]`
  so turning it off restores it, and installs a D-Bus activation file for
  `org.freedesktop.FileManager1`, then asks the bus to `ReloadConfig` (it read its service
  files at start and would otherwise activate Dolphin). D-Bus starts
  `rustyfiles --file-manager-service` (`fm_service`, no UI): `ShowItems` /
  `ShowItemProperties` start `rustyfiles /select,<file>`, `ShowFolders` `rustyfiles <folder>`
  (one per folder), which opens a window or goes to the running one; it exits after 60 s idle.
  A file manager that is already running keeps the name (Dolphin doesn't allow replacement).
  Tried end to end in a private bus session with scratch XDG folders
  (`cargo run -p rf-linux --example default_fm -- enable <exe> | disable | status`).
- ~~Tray and notifications~~ **done** (2026-11-09): `rf-linux` `tray` keeps rf-win's API
  (`show` / `hide` / `notify` / `set_handler`). The icon is a StatusNotifierItem from `ksni`
  (async-io + blocking, on the zbus we already have: no tokio) with the app icon as pixmaps
  and an Open / New window / Quit menu; it stays registered across bar restarts.
  Notifications call `org.freedesktop.Notifications.Notify` directly (no `notify-rust`).
  Both run on one worker thread. "Keep running in the background" and "Notify when long
  operations finish" are shown on Linux again; "rustyfiles isn't in front" asks GPUI whether
  one of our windows is active (Wayland has no foreground-window query). DirectLook's own
  tray icon stays Windows-only. Tried against Noctalia: the icon registers, Activate and
  the menu items reach the app, background mode keeps the process after the last window
  closes, Quit exits (`cargo run -p rf-linux --example tray -- [notify] [secs]`).
- ~~Removable drives and safe removal~~ **done** (2026-11-09): `rf-linux` `udisks` talks to
  UDisks2 on the system bus. `drives::unmounted_drives()` lists file systems UDisks doesn't
  hide (not recovery/reserved partitions or swap) that aren't mounted; their `root` is the
  device (`/dev/sdb1`), named "465 GB Volume" without a label, with no usage bar.
  `FolderView::navigate` mounts a block device and opens the mount point instead, so the
  sidebar, Home and anything else that opens it all mount. The sidebar menu has Unmount (any
  `/dev` mount but `/`, `/home`, `/boot`…) and Eject (removable / optical): every window
  lets go first (`workspace::release_mount`: panes go Home, previews close), then every file
  system on the drive is unmounted (an unlocked LUKS volume is locked again), and the drive
  ejected or powered off; a disk image is detached. "Busy" names the programs holding it
  (`udisks::users_of`, from `/proc`). `drive_mask` hashes `/sys/class/block` too, so a
  stick shows up before anything mounts it. Not done: unlocking LUKS (needs a passphrase
  prompt), and an eject button on the sidebar row. Tried with a FAT image on a loop device
  (`cargo run -p rf-linux --example udisks -- list | mount <dev> | unmount <dir> | eject <dir>`);
  the sidebar showed it as unmounted; clicking it and the menu items weren't tried by hand.
  **Network shares** (2026-11-11): the sidebar lists them under Network (Linux only; Windows
  keeps mapped drives with the others), before the network locations. `list_drives` reads
  `/etc/fstab`'s network entries: an `autofs` mount at one of their mount points is a systemd
  automount (`x-systemd.automount`) holding its place, listed as that share; entries not
  mounted at all are listed too; shares come last, in fstab's order. Before, an automounted
  share showed only once something had opened it, and `drive_mask` hashed only `/dev`
  mounts, so a share mounting later never refreshed the sidebar: it now hashes network and
  autofs mounts too. `slow_to_read` counts `autofs` (opening mounts it, up to its
  `x-systemd.mount-timeout`). `cargo run -p rf-linux --example drives` prints the list
  without touching any share.
- ~~Network and FTP~~ **done** (2026-11-09), through the desktop's network layer instead of
  our own SMB/FTP code: `rf-linux` `remote` mounts `smb://`, `sftp://`, `ftp(s)://`, WebDAV,
  `nfs://`… (and `\\server\share`, `//server/share`) with kio-fuse (`org.kde.KIOFuse`
  `mountUrl`, on the session bus) or else gvfs (`gio mount`, then its `local path`), and
  returns the local folder. `FolderView::navigate` (and a view created at such a location:
  command line, new tab) mounts and goes there, like unmounted drives. kio-fuse can't show
  KIO's password prompt ("Operation cancelled"), and a refused SMB login shows up as EPERM
  on the folder: both are `NeedsLogin`, and the app asks for the user name, then the
  password, and retries with them in the URL (kio-fuse keeps the password out of the path
  and its messages; logs only get the URL without it). `network_shortcuts` reads network
  bookmarks from KDE's `user-places.xbel` and GTK's `bookmarks`; pinning a folder inside a
  mount pins its URL (`remote_url`: kio-fuse `remoteUrl`, gvfs `gio info`), since the local
  path changes between sessions. rf-linux `ftp` stays a stub: `ftp://` never reaches it.
  The address bar shows a network folder by its address (setting `address_shows_url`, on by
  default; Settings → Files and folders): `remote::mount_url` reads it off the mount path
  (kio-fuse `…/kio-fuse-X/<scheme>/<authority>/…`, gvfs `…/gvfs/smb-share:server=…,share=…`,
  no D-Bus), the first crumb is the address and leads to the mount's root, and editing the
  address starts from the URL. Pressing Enter on it reopens the same folder: `mount` looks
  for kio-fuse's folder of that server first, since kio-fuse would otherwise ask for the
  password again. `smb://server` share lists depend on the server allowing it. Tried with a local
  pyftpdlib server (anonymous, then with a login), the app opening
  `ftp://user:pass@127.0.0.1:2122/pub` from the command line, and the sign-in prompt for
  the same URL without a login
  (`cargo run -p rf-linux --example remote -- <url>`).
- ~~File dialogs~~ **done** (2026-11-09): `dialogs::pick` (Settings: background image,
  code editor, import / export settings) shows the desktop's chooser through the
  xdg-desktop-portal FileChooser (`ashpd` `file_chooser`, blocking on the shell worker
  thread). Windows-style filters become portal globs that match either case; the code
  editor pick has no filter on Linux (programs have no `.exe`). Tried with the GTK portal on
  Hyprland: the save dialog came up with the name, folder and filter
  (`cargo run -p rf-linux --example pick -- open | folder | save`). rustyfiles *as* other
  apps' file dialog (rf-dialog) is still not ported: that would be a portal backend.
- ~~Drag out to other apps~~ **done** (2026-11-09; needs a hand check): GPUI 0.3.11 can hand
  an in-app drag to Wayland when the pointer leaves the window (`external_drag_payload`;
  X11 isn't supported by GPUI). Before this, dragging files did nothing on Linux, not even
  between panes: Windows routes every file drag through a shell OLE drag and drops come
  back as `ExternalPaths`. Now each row is a GPUI drag source (`FolderView::draggable`,
  list, grid and column views) whose value is an `ExternalPaths` of just its own path, so
  the 21 existing drop handlers keep working; when the drag starts, the selection it stands
  for is recorded (`file_drag::begin`) and drops expand it back (`file_drag::take` in
  `drop_into`, `drop_files`, `pin_dropped`). Leaving the window offers the whole selection
  as `text/uri-list`. Windows keeps its OLE drag (`start_drag_out` returns early on Linux).

### Linux polish

- **Drive names** (2026-11-09): Linux has no drive letters, so "Local Disk (C:)" became
  "Local Disk (:)". `rf-linux`'s `Drive` now carries the file system label
  (`/dev/disk/by-label`) and `source` (device or share), and Settings → Files and folders →
  "Show drives as" picks the name: Automatic (label, else "share (server)" for network
  shares, else the mount point's folder; `/` is "File System"), mount point, last folder, or
  device / share. `places::drive_name` has a Linux version; Windows' is unchanged.
- **Windows-only settings and links hidden** (2026-11-09). `settings_view::windows_only(row)`
  gives a hidden element (`display: none`: no space, no gap) on Linux, so upstream's
  `vec![…]` lists keep their shape; other spots use `cfg!(windows)`. Hidden: shell
  extensions submenu, Windows Search index, protected system files, Libraries, the OneDrive
  sync column, DirectLook over Explorer / in the taskbar, the GPU/CPU renderer switch, the
  ~~tray and notifications~~ (back since Phase 3), default file manager, archives and Open/Save dialogs
  (Phase 3), Velopack updates; "Show more options", Share, "Windows properties…", the
  Hidden checkbox (a dot name on Linux). Relabelled: background material (Translucent /
  with blur / Solid: GPUI's Linux windows are see-through unless opaque, blurred on request),
  "Follow system" animations (GNOME's `enable-animations` via gsettings, else on), the
  fast-copy description, "Cloud folders". Made to work: start at login (XDG autostart
  `.desktop`), Read-only in Properties (write permission), the sidebar's Properties (our own
  window). "Recycle Bin" reads "Trash" through a small Linux table in `i18n::t` (Turkish and
  English; a test fails if upstream rewords a key).
- Later: Windows' "Recent" on the home page reads `%APPDATA%\Microsoft\Windows\Recent`;
  Linux has `~/.local/share/recently-used.xbel`.

### Not ported

The taskbar preview (`--bar-preview`, upstream 2026-11-08), jump lists, Windows Libraries, Windows Search index (maybe later: Baloo/Tracker/plocate),
rf-dialog (an Open/Save dialog for other apps would be an xdg-desktop-portal backend: a
separate project), Velopack updates (ship a PKGBUILD/AUR package, then Flatpak/AppImage),
the hybrid GPU/CPU renderer, critical-error-dialog suppression, `keep_gdiplus_alive`.

## Upstream syncs

Merge `upstream/main` into a `sync/upstream-<version>` branch (merge commit, never squash).
New `rf-win` API that the app calls needs a Linux counterpart in `rf-linux` (the build says
which); new `rf-win` examples get the `support/<name>_win.rs` wrapper with a stub `main`;
new features need checking for Windows-only assumptions the compiler can't see (`\\`
separators, case-insensitive paths, drive letters). `README.md` is this fork's own (Linux only, since 2026-11-11): when upstream's README
changes, keep ours (`git checkout --ours README.md`) and carry over anything that matters
on Linux, such as a new feature or shortcut.

- **0.4.0** (2026-11-09, 52 commits). Linux stand-ins: `fastcopy` journals and resume
  (since ported: see the fastcopy row), `preview_host::mf_decodes`
  (false: DirectLook decodes 4K on the CPU as before). The speed test (races File Explorer)
  is Windows only (`bench_view_linux.rs` keeps the page empty and hidden). Fixed for Linux:
  adaptive layouts recognised Pictures / Videos subfolders with a `\\` separator. Works on
  Linux unchanged: hex and e-mail views in DirectLook, adaptive layouts (Pictures as a grid,
  Downloads in date groups), Sort by Size with measured folders, paste selecting what
  arrived, the `rustdecoder` crate (builds; `search-ms:` links parse too, though nothing on
  Linux sends them). Follow-ups: copy journals and resume
  in rf-linux's engine (done); the name index on Linux (done, below).
- **Name index on Linux** (2026-11-09, after 0.4.0). `rf-linux/src/nameindex.rs` is
  upstream's `rf-win/src/nameindex.rs` function by function, in the same order (compare the
  two files on an upstream sync). Not shared, because the differences run through all of
  it: names stored as raw bytes (non-UTF-8 names round-trip), folders found by their exact
  name (upstream folds case: "Foo" and "foo" would be one folder), `read_dir` + the entry
  type instead of `FindFirstFileExW`, dot names hidden, symlinks listed but never entered,
  other mounts inside a root (FUSE cloud folders, shares, another disk) listed but not walked
  (a search inside one walks the disk), the folder searched isn't its own hit (upstream
  returns it), and Linux's exclusions (`node_modules`, `.git`, `.cache`, `.cargo`, `.npm`…,
  anything with a `CACHEDIR.TAG`, the user's folders; case-sensitive). Roots
  (`indexer::local_roots`): the home folder plus fixed local disks (ext4, btrfs, xfs…)
  mounted outside it and outside `/`; never `/`, network, FUSE or removable mounts. Index
  files go to `~/.cache/rustyfiles/index` (`$RUSTYFILES_CONFIG_DIR/index` when set).
  Watching: raw inotify, one watch per indexed folder, added by the walk itself just before
  it lists the folder (a recursive `notify` watch would walk the tree a second time and
  watch the left-out caches). It takes at most half of `max_user_watches`; a root with more
  folders, or whose event queue overflowed, is walked again every 30 minutes. The disk-walk
  search (`rf-linux/src/search.rs`) was still a stub and is now rf-win's walk on
  `read_dir` (only hits are `stat`ed). The Settings card, the exclusions list (folder picked
  through the portal) and the context menu's "Leave out of the search index" show on Linux.
  `cargo run --release -p rf-linux --example search_bench -- <folder> <query>` compares
  the two; on this machine: home folder (525k entries, built in 4 s, 22 MB) 12 ms against
  0.25–1.2 s for the walk; `/usr` (361k) 3 ms against 35–64 ms (warm cache). The walk finds
  a few more hits: those inside the left-out folders.
- **main @ a1d7ca3** (2026-11-09, 89 commits after 0.4.0). Ported: `diskusage` (the "Show
  disk usage" tab: rf-win's tree with a `read_dir` walk that skips symlinks and stays on one
  file system, block size from `statvfs`, case-sensitive names; home folder here: 1.07 M
  entries in 1.0 s, 52 MB, totals equal to `du -sxbl`; `examples/diskusage.rs`), and
  `foldersize`'s remembered sizes (`foldersizes.bin` in `~/.local/share/rustyfiles`, shown as
  provisional and measured again). Fixed on the way: the folder size cache keyed paths
  case-insensitively and `invalidate` looked for `\` below a folder, so on Linux it never
  forgot the folders inside. Stand-ins: `shellfolder` (phones, Libraries: Windows shell
  namespace; Linux mounts phones as folders), DirectLook's rustdecoder route
  (`rd_video_linux.rs`: never taken, rustdecoder plays through Direct3D 11 and WASAPI). The
  new rf-win and rustdecoder examples are wrapped as Windows only; rustdecoder's test clips
  used ffmpeg's `-top`, an input-only option since ffmpeg 9, now `setfield`. Works on Linux
  unchanged: rustdecoder's containers (MXF, MPEG-TS/PS, AVI, fragmented MP4) and CPU
  decoders (ProRes, DNxHD), session tabs keeping the Recycle Bin, shared extension strings.
- **Name index memory** (2026-11-09, upstream `c8e1e57` + `b593628`). Same layout as
  upstream now: each different name stored once in a name table (raw bytes, as before),
  entries keep a 4-byte name id (9 bytes an entry), folder and name lookups are 4-byte id
  tables instead of keyed `HashMap`s, walk workers add names to the table as they list (a
  tree's raw names are never held twice), searching matches each different name once. A
  folder's exact name is its name id, so the case-sensitive folder lookup is (parent, name
  id); ids are kept unique, a file listing a name twice is folded on loading. Index files
  are format 2 (`RFNIXL\0\x02`), read straight into the arrays, written little-endian on any
  machine; format 1 files (`~/.cache/rustyfiles/index/*.rfidx` from before) convert on
  loading, and the start-up walk saves them as format 2. Not taken: the NTFS change journal
  (`5bc4945`, `8981033`) and the folder file ids it needs, which is all `b593628` counts
  (Linux keeps following changes with inotify). Hashing a folder's names happens before the
  walk's shared lock. Measured with `search_bench -- <folder> readme 0` (warm cache) and
  `index_mem` on the real saved file:

  | | entries | different names | index | process | query | build (warm) |
  |---|---|---|---|---|---|---|
  | home folder, before | 528,817 | — | 22.8 MB | 48 MB | 3.4 ms | 40 ms |
  | home folder, after | 528,817 | 314,634 | 20.8 MB | 32 MB | 3.0 ms | 76 ms |
  | `/usr`, before | 361,424 | — | 10.8 MB | 24 MB | 1.6 ms | 25 ms |
  | `/usr`, after | 361,424 | 210,727 | 9.0 MB | 17 MB | 1.2 ms | 38 ms |

  The index itself shrinks less than upstream's (C: 150 → 61 MB) because Linux leaves out
  the folders where names repeat most (`node_modules`, `.git`, `.cargo`, `target/`…): here
  60% of names are different, against 10% on upstream's drive. The process drops a third, as
  build workers no longer hold every raw name until the walk ends. The saved home index goes
  from 21.5 to 18.6 MB; the old file converts in 0.06 s. A warm-cache build takes longer
  (interning under one lock); a real walk is disk-bound, seconds, at nice 10.

- **main @ f25a485** (2026-11-10, 64 commits after a1d7ca3). Ported: folder views patched with
  what changed instead of listed again (`watch::watch_dir_changes` names the changed entries
  from inotify, a lost event or the folder itself asks for a full listing;
  `listing::stat_entry` reads one entry as the listing would), the quick search's ranked index
  search (`nameindex::search_ranked`, and hidden folders count only above the folder searched,
  as upstream), `drives::slow_to_read` (network, FUSE except `fuseblk`, removable and optical
  mounts: DirectLook opens those off the UI thread), SHA-384 and `process_age_ms`
  (`/proc/self/stat`). Shared with rf-win through `#[path]`, as they have no Windows code:
  `hash`, `apppkg` (MSIX, APK, IPA and NuGet info cards), `elf` (DirectLook's card for Linux
  programs) and `design` (CorelDRAW, Illustrator, Krita… embedded previews). Quick search
  (Alt+Space on Windows): Wayland has no global hotkeys, so `hotkey` holds nothing and Settings
  says to bind `rustyfiles --quick-search` in the desktop (a running rustyfiles takes it over);
  apps come from the `.desktop` files (`apps`, icons drawn from them), bookmarks from the
  Chromium family's `~/.config` profiles (Chrome, Chromium, Edge, Brave, Vivaldi). The tray
  menu got "Settings". Stand-ins: `taskbar_progress` (a later step could use the Unity
  LauncherEntry D-Bus API), `shell_window`, Zone.Identifier "Unblock" (never blocked), Media
  Foundation events, the Open/Save dialog's z-order checks. DirectLook's own tray card stays
  hidden. Fixed for Linux: opening a file in an already open folder compared paths without
  case.
- **v0.6.0 @ aac27ce** (2026-11-11, 4 commits after f25a485). Upstream's Properties window
  became Windows' property sheet, in tabs. On Linux it shows General (with "Opens with" and
  its Change button, which opens the desktop's app chooser), Details and Hashes, and a
  drive's page for its mount point (Linux drives have no letters: `/` is the root, and a
  folder that is a mount point gets the drive page). Attributes: only read-only, as before
  (hidden is a dot name; archive, indexing, compression and encryption are Windows'), with
  `fileinfo::set_attrs` / `set_attrs_tree` turning it into the write permission, and the
  plain "Read-only" label for folders (on Linux it stops changes in the folder itself).
  Windows-only, so hidden on Linux: Compatibility, Security (Windows ACLs), Digital
  signatures, Sharing, Customize (desktop.ini icons), Disk Cleanup and the Windows
  properties links. rf-linux has the same types for them (`compat`, `security`,
  `signature`, `folder_custom`, `shortcut::Details`, the new `PeInfo` fields) with reads that
  find nothing; `security::read` gives the owner from /etc/passwd for the Details tab.
  `breakable` wraps paths after `/` too. The shared `apppkg.rs` names its new `vector`
  submodule with `#[path]`: included from rf-linux, a plain `mod vector;` would be looked for
  in `src/`.

## Packaging

**Arch** (2026-11-09): `packaging/arch/PKGBUILD` builds `rustyfiles-git` from the GitHub
repo's latest commit (`pkgver`: the workspace version, commit count and commit, `0.3.0.r96.g8a667b8`; the GitHub repo has no tags). Both binaries go
to `/usr/lib/rustyfiles/` (DirectLook is looked up next to rustyfiles, and `current_exe`
resolves the `/usr/bin/rustyfiles` symlink), plus `packaging/linux/rustyfiles.desktop`
(`Exec=rustyfiles %U`: launchers pass `file://` URIs, which the app now decodes, and
`smb://`… addresses), an AppStream metainfo file, and the SVG / 256 px icons in hicolor.
Runtime libraries: alsa-lib, libxcb, libxkbcommon(-x11), and wayland, vulkan-icd-loader,
libglvnd, fontconfig, freetype2 (GPUI loads some at runtime). Everything else is an
optdepend: poppler, libavif, libheif, ffmpeg, kio-fuse / gvfs-smb, udisks2,
xdg-desktop-portal, git. The default-file-manager entry (same name, in the user's data dir)
uses the installed icon when there is one. Not done: an AUR upload (needs an AUR account),
Flatpak (needs portal support for everything outside the sandbox, which Phase 3 mostly
uses).

**Debian / Fedora** (2026-11-10): `packaging/linux/nfpm.yaml` describes one package for
both formats, with the same layout as the Arch one; `packaging/linux/build-packages.sh` runs
[nfpm](https://nfpm.goreleaser.com) on binaries already built and writes `dist/`. Version:
`0.5.2+git300.gabc1234` (workspace version, commit count, commit). The
`linux-packages.yml` workflow builds both on Ubuntu 22.04 (x86_64 and aarch64 runners), so
they need glibc 2.35+ (the .deb's `libc6 (>= …)` is the newest glibc symbol the binaries
use), installs each in a clean `debian:12` / `fedora:latest` container and checks that
`ldd` finds every library, then uploads them as artifacts (on `main`, on PRs that touch
the packaging, by hand). The .rpm depends on library names (`libEGL.so.1()(64bit)`…)
rather than package names, so Fedora and openSUSE both resolve it; the .deb accepts
`libasound2 | libasound2t64` (Ubuntu 24.04's rename). GPUI loads Wayland, EGL, Vulkan and
fontconfig at run time, so those are listed by hand.

**Releases** (2026-11-11): `linux-packages.yml` also builds the PKGBUILD in an
`archlinux:base-devel` container (`RUSTYFILES_SOURCE` points makepkg at the checkout
instead of GitHub) and installs it, and it can be called with a `ref`.
`linux-release.yml` runs it for a pushed `v*` tag and attaches every package to that tag's
release; a tagged build is versioned with the tag alone (`0.6.0`). Upstream's `release.yml`
(Windows, Velopack) runs on the same tag and creates the release, so the Linux job waits up
to an hour for it and creates it only if it never appears. Not done: an apt/dnf repository
(COPR, OBS).

**Making a release** (2026-11-11): put the new tag in `packaging/linux/RELEASE` and its notes
in `packaging/linux/release-notes.md`, in a PR. When it lands on `main`,
`linux-release.yml` creates the tag at that commit and a draft release with the notes,
builds and attaches every package, then publishes the release; a failed build leaves the
draft (run the workflow by hand with the tag to finish it). A tag pushed by CI's token
doesn't start upstream's Windows `release.yml`, so these releases are Linux only. Tags are
`v<upstream version>-linux.<n>` (`v0.5.2-linux.1`): they don't clash with upstream's tags,
and the packages sort right (`0.5.2~linux.1` comes before the `0.5.2+git…` builds from
`main` after it, and before `0.6.0~linux.1`).

**Flatpak** (2026-11-11): `packaging/flatpak/io.github.benjaminfenech.rustyfiles.yml`
(Freedesktop 26.08, rust-stable; `linux-packages.yml` builds a bundle per architecture in
Flathub's CI image, and releases attach them). A file manager can't live in a tight
sandbox, so it has `--filesystem=host`, `org.freedesktop.Flatpak` (host commands),
UDisks2, GVfs, notifications, the tray watcher and `FileManager1`. rf-linux's `sandbox`
module makes it act like a native install: at startup `adopt_host_dirs` points the XDG
folders back at the host's (trash, mimeapps.list, bookmarks, user dirs, thumbnails and
rustyfiles' settings are the same files), adds the host's and Flatpak's data dirs
(`/run/host/usr/share`, the Flatpak exports) for desktop entries and icons, and moves
`TMPDIR` to `$XDG_RUNTIME_DIR/app/<id>` (host tools write their output there). `tool()`
runs git, gio, gsettings, poppler, avif/heif tools and thumbnailers on the host through
`flatpak-spawn --host --watch-bus` (a timeout's kill ends the host process too) when the
host has them, else the sandbox's: the runtime has its own ffmpeg, gio and gsettings, but
they see the sandbox (no GVfs mounts, a private settings store, fewer codecs). rf-player
does the same for ffmpeg/ffprobe. `launch()` starts apps,
terminals, xdg-open and the IDE on the host without `--watch-bus` (they outlive
rustyfiles). A program neither side has still fails with `NotFound`, so the "not
installed" paths are unchanged. Desktop entries rustyfiles writes (default file manager,
its D-Bus service, autostart) run `flatpak run --command=… <id>`; the main window's
Wayland app id is the Flatpak id (its desktop entry); the tray registers without its own
`StatusNotifierItem-…` name (the sandbox can't own it). The build downloads crates with
network access; Flathub would need `cargo-sources.json` instead.

## Testing

- CI: add a `ubuntu-latest` job (build + `cargo test -p rf-core -p rf-archive -p rf-linux`)
  next to the Windows build, so Linux changes never break Windows.
- rf-linux unit tests in temp dirs: listing with odd names (non-UTF-8 bytes, newlines),
  copy keeping permissions/xattrs, trash and restore, symlink loops in search.
- Manual checks on both Wayland (KDE/GNOME) and X11 for each phase.

## Open questions

1. Track upstream `gelenmedya/rustyfiles`, or treat this as a hard fork? (This plan assumes
   tracking it: that's why the Windows code stays untouched.)
2. Which desktop is the main target: KDE Plasma or GNOME? It decides theming and which
   portal behaviors to test first.
3. Is DirectLook working only inside the app acceptable on Wayland?
4. Licensing and branding: the README, download links and Discord point at upstream. Should a
   Linux build carry its own name and links?

Already checked: the git dependencies (`gelenmedya/rarchive`, the `storytold/filmcraft`
codecs) compile on Linux unchanged.
