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
copy/paste inside rustyfiles), **icons and thumbnails done**, **menus drawn in-window**.
Next: "Open with", clipboard exchange with other apps, modifier keys from GPUI, the path
audit.


| Module | Linux implementation |
|---|---|
| listing, search, foldersize | `std::fs::read_dir` + `std::os::unix` metadata; hidden = leading dot; skip symlink loops (track dev+inode) instead of junctions |
| known, localized, libraries | `dirs` + `xdg-user-dirs`; drop localized names and libraries |
| drives, volume | `/proc/self/mountinfo` + `statvfs`; filter pseudo filesystems |
| watch | `notify` already uses inotify; just ungate |
| thumbs | ✅ `xdg_icons`: shared-mime-info (by extension) → the desktop's icon theme per the Icon Theme spec (theme from kdeglobals / GTK settings / gsettings, inherited themes, hicolor, pixmaps), SVGs drawn with resvg. `xdg_thumbs`: the shared `~/.cache/thumbnails` (read and written, `Thumb::MTime` checked, failures in `fail/rustyfiles`), images and SVGs decoded here, other types through the installed `.thumbnailer`s with a 15 s limit. `cargo run -p rf-linux --example icon_lookup` / `--example thumbnail` to check a system |
| shell::open, open_with, open_terminal | `xdg-open` / `gio launch`; `.desktop` files + `mimeapps.list` for "Open with"; `$TERMINAL` or `x-terminal-emulator` fallback |
| fastcopy, fastdelete, fileops | ✅ rf-win's engine and tests, ported: `copy_file_range` in 8 MB chunks (read/write fallback), modes and times kept, `renameat2(RENAME_NOREPLACE)`, same device = rename. `fileops` is the fallback engine (symlinks copied as links). Extended attributes and ACLs are not copied yet |
| recycle | ✅ freedesktop Trash spec, our own: home trash + per-disk `.Trash/$uid` / `.Trash-$uid`, list / restore / empty, undo of a delete. The UI still says "Recycle Bin" (renaming needs new translations) |
| clipboard | ◐ inside rustyfiles only (in-process). Still to do: `text/uri-list` + `x-special/gnome-copied-files` (cut vs copy) on the system clipboard; GPUI can only write text and images, so `wl-clipboard-rs` (Wayland data-control) and an X11 path |
| git, hash, encoding, complete | mostly portable: remove `creation_flags(CREATE_NO_WINDOW)` and Windows-path handling |
| single_instance | Unix socket in `$XDG_RUNTIME_DIR` (or a D-Bus name) |
| accent, sysparams, window::set_dark_mode | xdg-desktop-portal Settings (`color-scheme`, `accent-color`) via `ashpd`; GPUI window appearance |
| shortcut | `.desktop` files and symlinks instead of `.lnk` |
| props, fileinfo | `statx` for birth time; media properties from the existing decoders (drop the Windows property system) |
| ctxmenu | no native shell menu exists on Linux: use the app's own menu (`fluent_menu`/`ctx_menu`) with Open with, Terminal, Trash, Properties, plus custom actions later |
| sta | not needed: a plain worker thread pool; keep `run()` so call sites compile |

Names that aren't valid UTF-8 are shown lossily by `listing` today; keep the raw bytes so they
still open and rename.

rf-app path audit: drive letters, `\\server` UNC roots, backslash separators, case-insensitive
comparisons, `Location::from_shell_name`. Linux paths are case-sensitive and `/` is the
only root.

Done when: you can browse, copy, move, rename, trash, search and open files on Wayland and
X11 without freezes, including on a slow NFS/SMB mount (the "never waits on the disk" promise
still holds).

### Phase 2: DirectLook and media (about 2–3 weeks)

- **Global Space key over other apps does not work on Wayland** (no global keyboard hooks,
  no foreground-window inspection). On Linux DirectLook works inside rustyfiles only;
  `look/hook.rs` and `look/target.rs` stay Windows-only. Optional later: an X11-only hook
  or a desktop-portal GlobalShortcuts binding.
- `look/decode.rs`: drop WIC; HEIC/AVIF via `libheif-rs`, JXL via `jxl-oxide`, RAW via
  `rawler` or the embedded JPEG preview.
- `look/pdf.rs`: `pdfium-render` (bundled pdfium) or poppler-glib.
- `preview_host` (Windows preview handlers): no equivalent; leave out.
- DirectLook info card (from upstream, 2026-11-08): `pe` shows only the architecture on Linux;
  read the version strings from the `VS_VERSIONINFO` resource ourselves. `cert` returns
  nothing on Linux; decode with an X.509 crate (`x509-parser`).
- rf-player: present decoded frames as a GPUI image texture (simplest), and later a
  wgpu/Vulkan presenter if it is too slow. Audio via `cpal` (PipeWire/ALSA).

### Phase 3: desktop integration (about 2 weeks, can be split)

- Default file manager: install `rustyfiles.desktop`, `xdg-mime default … inode/directory`,
  and implement the `org.freedesktop.FileManager1` D-Bus interface (`ShowItems`,
  `ShowFolders`) so "Show in folder" from other apps lands here. Replaces `default_fm`.
- Tray and notifications: StatusNotifierItem (`ksni`) + `notify-rust`.
- Removable drives and safe removal: UDisks2 over D-Bus (`zbus`) for mount, unmount, eject.
- Network: browse mounted gvfs paths (`/run/user/$UID/gvfs`) first; native SMB later.
- FTP: `suppaftp` instead of WinINet.
- File dialogs: xdg-desktop-portal FileChooser (`ashpd`).
- Drag out to other apps: depends on GPUI's Linux drag-and-drop support; check first, it may
  need upstream work.

### Not ported

The taskbar preview (`--bar-preview`, upstream 2026-11-08), jump lists, Windows Libraries, Windows Search index (maybe later: Baloo/Tracker/plocate),
rf-dialog (an Open/Save dialog for other apps would be an xdg-desktop-portal backend: a
separate project), Velopack updates (ship a PKGBUILD/AUR package, then Flatpak/AppImage),
the hybrid GPU/CPU renderer, critical-error-dialog suppression, `keep_gdiplus_alive`.

## Packaging

Start with a PKGBUILD in `packaging/arch/` (target machine is CachyOS) and a
`rustyfiles.desktop` + icon. Flatpak later; it needs portal support for everything outside
the sandbox, which Phase 3 already uses.

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
