//! Opening a terminal in a folder, optionally running a command there.
//!
//! The terminal is the one picked in Settings, or (Automatic) the desktop's: `$TERMINAL`,
//! `xdg-terminals.list` through `xdg-terminal-exec`, KDE's and GNOME's own setting, then the
//! first common terminal installed. Each terminal names its "start in this folder" and
//! "run this" options differently; [`command_line`] knows the common ones and falls back to
//! xterm's `-e` (with the folder as the working directory) for the rest.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::sandbox;

/// A terminal to offer in Settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    /// What Settings stores and [`open`] runs: the program (`kitty`, `/opt/x/bin/term`).
    pub program: String,
    pub name: String,
}

/// Terminals tried, in this order, when the desktop names none.
const COMMON: [&str; 14] = [
    "x-terminal-emulator",
    "kgx",
    "ptyxis",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "ghostty",
    "kitty",
    "alacritty",
    "foot",
    "wezterm",
    "tilix",
    "terminator",
    "xterm",
];

/// The installed terminals (their desktop files), each program once.
pub fn installed() -> Vec<Terminal> {
    let mut out: Vec<Terminal> = Vec::new();
    for app in crate::xdg_apps::terminals() {
        let Some(program) = crate::xdg_apps::program(app) else { continue };
        if !out.iter().any(|t| t.program == program) && sandbox::in_path(&program) {
            out.push(Terminal { program, name: app.name.clone() });
        }
    }
    out
}

/// The terminal Automatic picks, when there is one.
pub fn automatic() -> Option<String> {
    let env = std::env::var("TERMINAL").ok().filter(|t| !t.trim().is_empty());
    let config = dirs::config_dir().unwrap_or_default();
    // xdg-terminal-exec reads xdg-terminals.list (and its desktop-specific variants)
    let xte_list = std::fs::read_dir(&config).into_iter().flatten().flatten().any(|e| e.file_name().to_string_lossy().ends_with("xdg-terminals.list"));
    let kde = std::fs::read_to_string(config.join("kdeglobals")).ok().and_then(|t| kde_terminal(&t));
    let gnome = gnome_terminal();
    let xte = "xdg-terminal-exec".to_string();
    let candidates = env
        .into_iter()
        .chain(xte_list.then(|| xte.clone()))
        .chain(kde)
        .chain(gnome)
        .chain(std::iter::once(xte))
        .chain(COMMON.iter().map(|s| s.to_string()));
    candidates.into_iter().find(|t| program_of(t).is_some_and(|p| sandbox::in_path(&p)))
}

/// `[General] TerminalApplication=kitty` in kdeglobals.
fn kde_terminal(text: &str) -> Option<String> {
    let mut general = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            general = line == "[General]";
        } else if general && let Some(v) = line.strip_prefix("TerminalApplication=") {
            return Some(v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

/// GNOME's `org.gnome.desktop.default-applications.terminal exec` (often `xdg-terminal-exec`).
fn gnome_terminal() -> Option<String> {
    if !std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_lowercase().contains("gnome")) {
        return None;
    }
    let mut c = sandbox::tool("gsettings", &[]);
    c.args(["get", "org.gnome.desktop.default-applications.terminal", "exec"]);
    let r = crate::tool::run(c, std::time::Duration::from_secs(3)).ok().filter(|r| r.ok)?;
    let v = String::from_utf8_lossy(&r.stdout).trim().trim_matches('\'').to_string();
    Some(v).filter(|v| !v.is_empty())
}

/// The program of a terminal setting that may carry options (`kitty --single-instance`).
fn program_of(setting: &str) -> Option<String> {
    setting.split_whitespace().next().map(str::to_string)
}

/// The terminal `choice` stands for: itself, or Automatic's pick when empty.
pub fn resolve(choice: &str) -> Option<String> {
    if choice.trim().is_empty() { automatic() } else { Some(choice.trim().to_string()) }
}

/// The shell line that runs `command` and then leaves an interactive shell open, so its output
/// stays readable and the terminal can be used on.
fn keep_open(command: &str) -> String {
    format!("{command}\nexec \"${{SHELL:-sh}}\"")
}

/// The argument list for `terminal` (a program, maybe with options) to start in `dir`, running
/// `command` there when given.
pub fn command_line(terminal: &str, dir: &Path, command: Option<&str>) -> Vec<String> {
    let mut words: Vec<String> = terminal.split_whitespace().map(str::to_string).collect();
    let prog = words.first().cloned().unwrap_or_default();
    let base = Path::new(&prog).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let d = dir.to_string_lossy().into_owned();
    let run: Option<Vec<String>> = command.map(|c| vec!["sh".into(), "-c".into(), keep_open(c)]);
    // how each one takes a folder, and what comes before the command
    let (cwd, before_cmd): (Vec<String>, Vec<&str>) = match base.as_str() {
        "kitty" => (vec!["--directory".into(), d], vec![]),
        "alacritty" => (vec!["--working-directory".into(), d], vec!["-e"]),
        "foot" | "footclient" => (vec![format!("--working-directory={d}")], vec![]),
        "wezterm" => (vec!["start".into(), "--cwd".into(), d], vec!["--"]),
        "konsole" => (vec!["--workdir".into(), d], vec!["-e"]),
        "gnome-terminal" | "kgx" | "ptyxis" | "mate-terminal" => (vec![format!("--working-directory={d}")], vec!["--"]),
        "ghostty" => (vec![format!("--working-directory={d}")], vec!["-e"]),
        "xfce4-terminal" => (vec![format!("--working-directory={d}")], vec!["-x"]),
        "tilix" | "terminator" => (vec![format!("--working-directory={d}")], vec!["-x"]),
        "xdg-terminal-exec" => (vec![format!("--dir={d}")], vec![]),
        // xterm, urxvt, st and others: started in the folder, `-e` before the command
        _ => (vec![], vec!["-e"]),
    };
    words.extend(cwd);
    if let Some(run) = run {
        words.extend(before_cmd.into_iter().map(str::to_string));
        words.extend(run);
    }
    words
}

/// Opens `choice` (or the automatic terminal, when empty) in `dir`, running `command` there
/// when given. Detached: the terminal outlives rfiles.
pub fn open(choice: &str, dir: &Path, command: Option<&str>) -> anyhow::Result<()> {
    let terminal = resolve(choice).ok_or_else(|| anyhow::anyhow!("No terminal found. Pick one in Settings → Advanced → Terminal."))?;
    let words = command_line(&terminal, dir, command.map(str::trim).filter(|c| !c.is_empty()));
    let (prog, args) = words.split_first().ok_or_else(|| anyhow::anyhow!("No terminal set"))?;
    let mut c: Command = sandbox::launch(prog);
    c.args(args).current_dir(existing(dir));
    crate::shell::spawn_detached(&mut c).map_err(|e| anyhow::anyhow!("{prog}: {e}"))
}

/// `dir`, or its nearest existing parent (the working directory must exist to spawn).
fn existing(dir: &Path) -> PathBuf {
    dir.ancestors().find(|a| a.is_dir()).unwrap_or(Path::new("/")).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(t: &str, cmd: Option<&str>) -> Vec<String> {
        command_line(t, Path::new("/home/u/My dir"), cmd)
    }

    #[test]
    fn each_terminal_gets_its_own_options() {
        assert_eq!(line("kitty", None), ["kitty", "--directory", "/home/u/My dir"]);
        assert_eq!(line("alacritty", Some("ls")), ["alacritty", "--working-directory", "/home/u/My dir", "-e", "sh", "-c", "ls\nexec \"${SHELL:-sh}\""]);
        assert_eq!(line("gnome-terminal", Some("ls"))[1..3], ["--working-directory=/home/u/My dir", "--"]);
        assert_eq!(line("/usr/bin/konsole", None), ["/usr/bin/konsole", "--workdir", "/home/u/My dir"]);
        assert_eq!(line("wezterm", Some("make"))[..5], ["wezterm", "start", "--cwd", "/home/u/My dir", "--"]);
        assert_eq!(line("foot", Some("make"))[..3], ["foot", "--working-directory=/home/u/My dir", "sh"]);
        assert_eq!(line("xterm", Some("make"))[..3], ["xterm", "-e", "sh"]);
        assert_eq!(line("xterm", None), ["xterm"]);
        assert_eq!(line("kitty --single-instance", None), ["kitty", "--single-instance", "--directory", "/home/u/My dir"]);
    }

    #[test]
    fn the_command_reaches_the_shell_unchanged() {
        let l = line("kitty", Some("echo \"$HOME\" && git log | head"));
        assert_eq!(l[3..5], ["sh", "-c"]);
        assert!(l[5].starts_with("echo \"$HOME\" && git log | head\n"));
    }

    #[test]
    fn kde_setting() {
        assert_eq!(kde_terminal("[General]\nTerminalApplication=kitty\n[KDE]\nx=1\n").as_deref(), Some("kitty"));
        assert_eq!(kde_terminal("[KDE]\nTerminalApplication=kitty\n"), None);
        assert_eq!(kde_terminal("[General]\nTerminalApplication=\n"), None);
    }

    #[test]
    fn a_missing_folder_starts_in_its_parent() {
        assert_eq!(existing(Path::new("/tmp/surely/not/here")), PathBuf::from("/tmp"));
    }
}
