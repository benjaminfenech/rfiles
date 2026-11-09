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
| listing, search, foldersize | `std::fs::read_dir` + `std::os::unix` metadata; hidden = leading dot; skip symlink loops (track dev+inode) instead of junctions |
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
separators, case-insensitive paths, drive letters).

- **0.4.0** (2026-11-09, 52 commits). Linux stand-ins: `fastcopy` journals and resume
  (since ported: see the fastcopy row), `indexer` /
  `nameindex` (the NTFS name index: never running, searches walk the disk as before; its
  Settings card and "Leave out of the search index" are hidden), `preview_host::mf_decodes`
  (false: DirectLook decodes 4K on the CPU as before). The speed test (races File Explorer)
  is Windows only (`bench_view_linux.rs` keeps the page empty and hidden). Fixed for Linux:
  adaptive layouts recognised Pictures / Videos subfolders with a `\\` separator. Works on
  Linux unchanged: hex and e-mail views in DirectLook, adaptive layouts (Pictures as a grid,
  Downloads in date groups), Sort by Size with measured folders, paste selecting what
  arrived, the `rustdecoder` crate (builds; `search-ms:` links parse too, though nothing on
  Linux sends them). Follow-ups: copy journals and resume
  in rf-linux's engine (done); the name index on Linux (ext4/btrfs walk + inotify).

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
uses), Debian/Fedora packages.

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
