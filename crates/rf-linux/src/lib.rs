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
pub mod apps;
pub mod cert;
pub mod clipboard;
pub mod complete;
pub mod ctxmenu;
pub mod default_fm;
pub mod dialog_reg;
pub mod dialogs;
pub mod diskusage;
pub mod dnd;
pub mod drives;
pub mod encoding;
pub mod fastcopy;
pub mod fastdelete;
pub mod fileops;
pub mod foldersize;
pub mod sysparams;
pub mod git;
// Shared with rf-win as they are (no Windows code in them): upstream's fixes arrive with each sync
#[path = "../../rf-win/src/hash.rs"]
pub mod hash;
#[path = "../../rf-win/src/apppkg.rs"]
pub mod apppkg;
#[path = "../../rf-win/src/elf.rs"]
pub mod elf;
#[path = "../../rf-win/src/design.rs"]
pub mod design;
pub mod journal;
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
pub mod hotkey;
pub mod udisks;
pub mod indexer;
pub mod nameindex;
pub mod remote;
pub mod sandbox;
pub mod jumplist;
pub mod wsearch;
pub mod preview_host;
pub mod props;
pub mod recycle;
pub mod search;
pub mod shell;
pub mod shellfolder;
pub mod shell_window;
pub mod shortcut;
pub mod single_instance;
pub mod sta;
pub mod taskbar_progress;
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
pub use listing::{ListError, list_dir, stat_entry};

/// Milliseconds since this process was created (start-up timing): `/proc/self/stat`'s start
/// time (clock ticks after boot) against the boot clock.
pub fn process_age_ms() -> u64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else { return 0 };
    // the command name may hold spaces and parentheses: fields count from the last ')'
    let Some(start) = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(19)).and_then(|f| f.parse::<u64>().ok()) else {
        return 0;
    };
    // SAFETY: plain calls filling a local struct
    let (ticks, now) = unsafe {
        let mut ts: libc::timespec = std::mem::zeroed();
        libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts);
        (libc::sysconf(libc::_SC_CLK_TCK).max(1) as u64, ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000)
    };
    now.saturating_sub(start * 1000 / ticks)
}

#[cfg(test)]
mod tests {
    #[test]
    fn process_age_is_plausible() {
        let a = super::process_age_ms();
        assert!(a > 0 && a < 3_600_000, "{a} ms");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(super::process_age_ms() >= a + 40);
    }
}
