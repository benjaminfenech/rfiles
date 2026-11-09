//! rf-linux: Linux platform layer. Same modules and functions as rf-win, so rf-app can use
//! either one as `platform`. Many functions are still stubs (see docs/LINUX_PORT.md).
//!
//! Every function here may block (disk, network, shell extensions). Never call them on the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![cfg(unix)]
#![allow(unused)]

/// Stands in for Windows handles (HWND and friends) in signatures shared with rf-win.
pub type Opaque = isize;

/// Screen rectangle with the same field names as Windows' `RECT`, so rf-app can build one
/// the same way on both platforms.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
pub mod accent;
pub mod cert;
pub mod clipboard;
pub mod complete;
pub mod ctxmenu;
pub mod default_fm;
pub mod dialog_reg;
pub mod dialogs;
pub mod dnd;
pub mod drives;
pub mod encoding;
pub mod fastcopy;
pub mod fastdelete;
pub mod fileops;
pub mod foldersize;
pub mod sysparams;
pub mod git;
pub mod hash;
pub mod known;
pub mod libraries;
pub mod ftp;
pub mod keys;
pub mod look;
pub mod listing;
pub mod localized;
pub mod network;
pub mod pe;
pub mod fileactions;
pub mod fileinfo;
/// Linux only: org.freedesktop.FileManager1 ("Show in folder") while we are the default.
pub mod fm_service;
pub mod udisks;
pub mod jumplist;
pub mod wsearch;
pub mod preview_host;
pub mod props;
pub mod recycle;
pub mod search;
pub mod shell;
pub mod shortcut;
pub mod single_instance;
pub mod sta;
pub mod removal;
pub mod rotate;
pub mod thumbs;
/// Linux only: running command-line tools.
pub mod tool;
pub mod tray;
pub mod watch;
pub mod window;
pub mod volume;
pub mod wide;
pub mod xdg_apps;
pub mod xdg_icons;
pub mod xdg_thumbs;
pub use drives::{Drive, DriveKind, list_drives};
pub use known::{KnownFolder, known_folders};
pub use listing::{ListError, list_dir};
