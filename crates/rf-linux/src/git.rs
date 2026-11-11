//! Git integration for the details view (per-file state column / colours) and the status bar
//! (branch, ahead/behind). Shells out to the user's `git` with `status --porcelain=v2 -z`
//! instead of linking libgit2: no extra binary size, and identical results to the command line
//! (ignore rules, submodules, sparse checkout, ...). Blocking: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use anyhow::{Result, bail};
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Ignored,
    Conflicted,
}
impl FileState {
    /// Priority when folding several descendants into one folder state.
    fn rank(self) -> u8 {
        match self {
            FileState::Conflicted => 4,
            FileState::Modified
            | FileState::Added
            | FileState::Deleted
            | FileState::Renamed => 3,
            FileState::Untracked => 2,
            FileState::Ignored => 1,
        }
    }
    /// Short Turkish label for tooltips / a state column.
    pub fn label(self) -> &'static str {
        match self {
            FileState::Modified => "Değiştirildi",
            FileState::Added => "Eklendi",
            FileState::Deleted => "Silindi",
            FileState::Renamed => "Yeniden adlandırıldı",
            FileState::Untracked => "İzlenmiyor",
            FileState::Ignored => "Yoksayıldı",
            FileState::Conflicted => "Çakışma",
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    /// Repository root the paths below are relative to.
    pub root: PathBuf,
    /// Current branch, or `HEAD (abc1234)` when detached.
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    /// Repo-relative path -> state. Untracked / ignored folders appear as one entry for the
    /// whole folder (git collapses them), not per file.
    pub files: HashMap<PathBuf, FileState>,
    /// Derived: repo-relative folder -> aggregated state of tracked-or-untracked descendants
    /// (ignored descendants do not propagate). Built by [`GitStatus::reindex`].
    pub dirs: HashMap<PathBuf, FileState>,
}
impl GitStatus {
    /// Rebuilds [`dirs`](Self::dirs) from [`files`](Self::files). Call after editing `files`.
    pub fn reindex(&mut self) {
        let mut dirs: HashMap<PathBuf, FileState> = HashMap::new();
        for (p, &st) in &self.files {
            if st == FileState::Ignored {
                continue;
            }
            let folded = match st {
                FileState::Conflicted => FileState::Conflicted,
                FileState::Untracked => FileState::Untracked,
                _ => FileState::Modified,
            };
            for a in p.ancestors().skip(1) {
                if a.as_os_str().is_empty() {
                    break;
                }
                dirs.entry(a.to_path_buf())
                    .and_modify(|cur| {
                        if folded.rank() > cur.rank()
                            || (folded.rank() == cur.rank()
                                && folded == FileState::Modified)
                        {
                            *cur = folded;
                        }
                    })
                    .or_insert(folded);
            }
        }
        self.dirs = dirs;
    }
    /// True when nothing differs from HEAD (ignored files do not count).
    pub fn is_clean(&self) -> bool {
        self.files.values().all(|&s| s == FileState::Ignored)
    }
}
/// Walks up from `dir` looking for a `.git` entry (directory, or file for worktrees and
/// submodules). Pure filesystem check, cheap enough for every navigation.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|a| a.join(".git").exists()).map(Path::to_path_buf)
}
fn git_cmd(root: &Path) -> Command {
    let mut c = crate::sandbox::tool("git", &[("GIT_OPTIONAL_LOCKS", "0"), ("GIT_TERMINAL_PROMPT", "0")]);
    c.arg("--no-optional-locks").current_dir(root).stdin(Stdio::null());
    c
}
/// Runs `git status` for the repository at `repo_root` (use [`repo_root`] to find it).
/// Errors when git is not installed or the command fails.
pub fn status(repo_root: &Path) -> Result<GitStatus> {
    let out = git_cmd(repo_root)
        .args([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=normal",
            "--ignored=traditional",
        ])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("Git bulunamadı (kurulu değil ya da PATH'te yok)")
            } else {
                anyhow::Error::new(e).context("git çalıştırılamadı")
            }
        })?;
    if !out.status.success() {
        bail!(
            "git status başarısız: {}", String::from_utf8_lossy(& out.stderr).trim()
        );
    }
    Ok(parse_porcelain_v2(repo_root, &out.stdout))
}
fn map_xy(kind: char, x: char, y: char) -> FileState {
    if kind == '2' {
        FileState::Renamed
    } else if x == 'D' || y == 'D' {
        FileState::Deleted
    } else if x == 'A' || y == 'A' {
        FileState::Added
    } else {
        FileState::Modified
    }
}
/// Parses `git status --porcelain=v2 --branch -z` output. Tolerant: unknown or malformed
/// records are skipped.
pub fn parse_porcelain_v2(root: &Path, raw: &[u8]) -> GitStatus {
    let text = String::from_utf8_lossy(raw);
    let mut st = GitStatus {
        root: root.to_path_buf(),
        ..Default::default()
    };
    let mut oid = String::new();
    let mut recs = text.split('\0');
    while let Some(rec) = recs.next() {
        if rec.is_empty() {
            continue;
        }
        if let Some(h) = rec.strip_prefix("# ") {
            if let Some(v) = h.strip_prefix("branch.oid ") {
                oid = v.to_string();
            } else if let Some(v) = h.strip_prefix("branch.head ") {
                st.branch = v.to_string();
            } else if let Some(v) = h.strip_prefix("branch.ab ") {
                for tok in v.split_whitespace() {
                    if let Some(n) = tok.strip_prefix('+') {
                        st.ahead = n.parse().unwrap_or(0);
                    } else if let Some(n) = tok.strip_prefix('-') {
                        st.behind = n.parse().unwrap_or(0);
                    }
                }
            }
            continue;
        }
        let mut chars = rec.chars();
        let kind = chars.next().unwrap_or(' ');
        let (state, path) = match kind {
            '1' => {
                let f: Vec<&str> = rec.splitn(9, ' ').collect();
                if f.len() < 9 {
                    continue;
                }
                let mut xy = f[1].chars();
                (map_xy('1', xy.next().unwrap_or('.'), xy.next().unwrap_or('.')), f[8])
            }
            '2' => {
                let f: Vec<&str> = rec.splitn(10, ' ').collect();
                let _orig = recs.next();
                if f.len() < 10 {
                    continue;
                }
                (FileState::Renamed, f[9])
            }
            'u' => {
                let f: Vec<&str> = rec.splitn(11, ' ').collect();
                if f.len() < 11 {
                    continue;
                }
                (FileState::Conflicted, f[10])
            }
            '?' => {
                match rec.get(2..) {
                    Some(p) => (FileState::Untracked, p),
                    None => continue,
                }
            }
            '!' => {
                match rec.get(2..) {
                    Some(p) => (FileState::Ignored, p),
                    None => continue,
                }
            }
            _ => continue,
        };
        let p = path.trim_end_matches('/');
        if !p.is_empty() {
            st.files.insert(PathBuf::from(p), state);
        }
    }
    if st.branch == "(detached)" {
        let short: String = oid.chars().take(7).collect();
        st.branch = if short.is_empty() {
            "HEAD".into()
        } else {
            format!("HEAD ({short})")
        };
    }
    st.reindex();
    st
}
/// `path` relative to `root`, comparing components case-insensitively (Windows).
fn rel_to(root: &Path, path: &Path) -> Option<PathBuf> {
    let mut rc = root.components();
    let mut pc = path.components();
    loop {
        match (rc.next(), pc.next()) {
            (None, rest) => {
                let mut out = PathBuf::new();
                if let Some(c) = rest {
                    out.push(c.as_os_str());
                }
                out.extend(pc.map(|c| c.as_os_str()));
                return Some(out);
            }
            (Some(a), Some(b)) if a.as_os_str().eq_ignore_ascii_case(b.as_os_str()) => {}
            _ => return None,
        }
    }
}
/// State of the entry `name` inside the listed folder `dir` (absolute path inside the repo).
/// Files report their own state. Folders report the aggregate of their descendants (any
/// changed descendant -> `Modified`, conflict wins, then `Untracked`); a folder inside an
/// untracked or ignored folder inherits that state. `None` = clean / not in the repo.
pub fn state_for_child(status: &GitStatus, dir: &Path, name: &str) -> Option<FileState> {
    let child = rel_to(&status.root, dir)?.join(name);
    if let Some(&s) = status.files.get(&child) {
        return Some(s);
    }
    for a in child.ancestors().skip(1) {
        if a.as_os_str().is_empty() {
            break;
        }
        if let Some(&s @ (FileState::Untracked | FileState::Ignored)) = status
            .files
            .get(a)
        {
            return Some(s);
        }
    }
    status.dirs.get(&child).copied()
}
fn spawn_err(e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        anyhow::anyhow!("Git bulunamadı (kurulu değil ya da PATH'te yok)")
    } else {
        anyhow::Error::new(e).context("git çalıştırılamadı")
    }
}
/// Runs git and returns the raw output (even on a non-zero exit).
fn run_git(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    git_cmd(root).args(args).output().map_err(spawn_err)
}
/// Runs git and returns stdout; on failure bails with `what: <stderr>`.
fn git_text(root: &Path, args: &[&str], what: &str) -> Result<String> {
    let out = run_git(root, args)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = if err.trim().is_empty() {
            String::from_utf8_lossy(&out.stdout)
        } else {
            err
        };
        bail!("{what}: {}", err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
/// A local or remote-tracking branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    /// `main` for local branches, `origin/main` for remote-tracking ones.
    pub name: String,
    pub is_remote: bool,
    /// The currently checked-out branch.
    pub is_head: bool,
    /// Short name of the upstream (`origin/main`), local branches only.
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}
/// Lists local branches (HEAD first, then by name) followed by remote-tracking branches
/// (by name). Symbolic `origin/HEAD` refs are skipped.
pub fn branches(root: &Path) -> Result<Vec<Branch>> {
    let text = git_text(
        root,
        &[
            "for-each-ref",
            "--format=%(HEAD)%00%(refname)%00%(upstream:short)%00%(upstream:track,nobracket)",
            "refs/heads",
            "refs/remotes",
        ],
        "Dallar listelenemedi",
    )?;
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 4 {
            continue;
        }
        let (is_head, refname, upstream, track) = (f[0].trim() == "*", f[1], f[2], f[3]);
        if let Some(n) = refname.strip_prefix("refs/heads/") {
            let (mut ahead, mut behind) = (0, 0);
            for part in track.split(',') {
                let part = part.trim();
                if let Some(v) = part.strip_prefix("ahead ") {
                    ahead = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = part.strip_prefix("behind ") {
                    behind = v.trim().parse().unwrap_or(0);
                }
            }
            local
                .push(Branch {
                    name: n.to_string(),
                    is_remote: false,
                    is_head,
                    upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
                    ahead,
                    behind,
                });
        } else if let Some(n) = refname.strip_prefix("refs/remotes/") {
            if n.ends_with("/HEAD") || !n.contains('/') {
                continue;
            }
            remote
                .push(Branch {
                    name: n.to_string(),
                    is_remote: true,
                    is_head: false,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                });
        }
    }
    local.sort_by(|a, b| b.is_head.cmp(&a.is_head).then_with(|| a.name.cmp(&b.name)));
    remote.sort_by(|a, b| a.name.cmp(&b.name));
    local.extend(remote);
    Ok(local)
}
/// What to do with uncommitted changes when switching branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutMode {
    /// Plain switch; fails when local changes would be overwritten.
    Normal,
    /// Stash (including untracked files), switch, then pop the stash.
    Stash,
    /// `--force`: throw away conflicting local changes.
    Discard,
}
enum Target {
    Local(String),
    /// Remote-tracking ref (`origin/foo`) without a local branch yet: create and track.
    Track(String),
}
fn ref_exists(root: &Path, full_ref: &str) -> bool {
    run_git(root, &["show-ref", "--verify", "--quiet", full_ref])
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn resolve_target(root: &Path, branch: &str) -> Result<Target> {
    if branch.starts_with('-') || branch.is_empty() {
        bail!("Geçersiz dal adı: {branch}");
    }
    if ref_exists(root, &format!("refs/heads/{branch}")) {
        return Ok(Target::Local(branch.to_string()));
    }
    if ref_exists(root, &format!("refs/remotes/{branch}")) {
        let local = branch.split_once('/').map(|(_, n)| n).unwrap_or(branch);
        if ref_exists(root, &format!("refs/heads/{local}")) {
            return Ok(Target::Local(local.to_string()));
        }
        return Ok(Target::Track(branch.to_string()));
    }
    bail!("Dal bulunamadı: {branch}")
}
fn switch_to(root: &Path, target: &Target, force: bool) -> Result<()> {
    let mut args: Vec<&str> = vec!["switch"];
    if force {
        args.push("--force");
    }
    match target {
        Target::Local(n) => args.push(n),
        Target::Track(r) => {
            args.push("--track");
            args.push(r);
        }
    }
    let out = run_git(root, &args)?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim();
    if err.contains("overwritten") || err.contains("would be lost") {
        bail!(
            "Dal değiştirilemedi: yerel değişiklikler çakışıyor. Değişiklikleri saklayın ya da atın.\n{err}"
        );
    }
    bail!("Dal değiştirilemedi: {err}")
}
/// Switches to `branch`. A remote-tracking name (`origin/foo`) creates and tracks local `foo`
/// (or just switches to it when it already exists).
pub fn checkout(root: &Path, branch: &str, mode: CheckoutMode) -> Result<()> {
    let target = resolve_target(root, branch)?;
    match mode {
        CheckoutMode::Normal => switch_to(root, &target, false),
        CheckoutMode::Discard => switch_to(root, &target, true),
        CheckoutMode::Stash => {
            let dirty = !git_text(
                    root,
                    &["status", "--porcelain"],
                    "git status başarısız",
                )?
                .trim()
                .is_empty();
            if dirty {
                git_text(
                    root,
                    &[
                        "stash",
                        "push",
                        "--include-untracked",
                        "-m",
                        "rfiles checkout",
                    ],
                    "Değişiklikler stash'e alınamadı",
                )?;
            }
            if let Err(e) = switch_to(root, &target, false) {
                if dirty {
                    let _ = run_git(root, &["stash", "pop"]);
                }
                return Err(e);
            }
            if dirty {
                git_text(
                    root,
                    &["stash", "pop"],
                    "Dal değiştirildi ama saklanan değişiklikler geri uygulanamadı (git stash listesinde duruyor)",
                )?;
            }
            Ok(())
        }
    }
}
/// Local check mirroring `git check-ref-format --branch` (without spawning git).
pub fn is_valid_branch_name(name: &str) -> bool {
    if name.is_empty() || name == "@" || name.starts_with('-') || name.starts_with('/') {
        return false;
    }
    if name.ends_with('/') || name.ends_with(".lock") || name.ends_with('.') {
        return false;
    }
    if name.contains("..") || name.contains("@{") || name.contains("//") {
        return false;
    }
    if name
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
    {
        return false;
    }
    name.split('/').all(|c| !c.starts_with('.') && !c.ends_with(".lock"))
}
/// Creates branch `name` (from `from`, default HEAD); optionally switches to it.
pub fn create_branch(
    root: &Path,
    name: &str,
    from: Option<&str>,
    checkout: bool,
) -> Result<()> {
    if !is_valid_branch_name(name) {
        bail!("Geçersiz dal adı: {name}");
    }
    if let Some(f) = from && (f.is_empty() || f.starts_with('-')) {
        bail!("Geçersiz başlangıç noktası: {f}");
    }
    let mut args: Vec<&str> = if checkout {
        vec!["switch", "-c", name]
    } else {
        vec!["branch", name]
    };
    if let Some(f) = from {
        args.push(f);
    }
    git_text(root, &args, "Dal oluşturulamadı").map(|_| ())
}
/// Deletes a local branch (`force` = `-D`, allows unmerged branches).
pub fn delete_branch(root: &Path, name: &str, force: bool) -> Result<()> {
    if name.is_empty() || name.starts_with('-') {
        bail!("Geçersiz dal adı: {name}");
    }
    let flag = if force { "-D" } else { "-d" };
    git_text(root, &["branch", flag, name], "Dal silinemedi").map(|_| ())
}
fn flush_line(cur: &mut Vec<u8>, kept: &mut Vec<String>, cb: &mut dyn FnMut(&str)) {
    if cur.is_empty() {
        return;
    }
    let line = String::from_utf8_lossy(cur).trim().to_string();
    cur.clear();
    if line.is_empty() {
        return;
    }
    cb(&line);
    if !is_progress_line(&line) {
        kept.push(line);
    }
}
fn is_progress_line(line: &str) -> bool {
    let l = line.strip_prefix("remote: ").unwrap_or(line);
    l.contains('%')
        && [
            "Receiving objects",
            "Resolving deltas",
            "Counting objects",
            "Compressing objects",
            "Writing objects",
            "Enumerating objects",
            "Updating files",
        ]
            .iter()
            .any(|p| l.starts_with(p))
}
/// Runs `cmd`, feeding every stderr line (split on `\r` and `\n`, so progress updates arrive
/// live) to `on_progress`. On failure the error carries the last meaningful output lines.
fn run_streaming(
    mut cmd: Command,
    what: &str,
    on_progress: &mut dyn FnMut(&str),
) -> Result<()> {
    use std::io::Read;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(spawn_err)?;
    let mut stdout = child.stdout.take().expect("stdout piped");
    let out_thread = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stdout.read_to_end(&mut v);
        v
    });
    let mut stderr = child.stderr.take().expect("stderr piped");
    let mut kept: Vec<String> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stderr.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\r' || b == b'\n' {
                        flush_line(&mut cur, &mut kept, on_progress);
                    } else {
                        cur.push(b);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    flush_line(&mut cur, &mut kept, on_progress);
    let status = child.wait().map_err(spawn_err)?;
    let out_bytes = out_thread.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let tail = |lines: Vec<String>| -> String {
        let skip = lines.len().saturating_sub(6);
        lines[skip..].join("\n")
    };
    let mut msg = tail(kept);
    if msg.is_empty() {
        let stdout_lines: Vec<String> = String::from_utf8_lossy(&out_bytes)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        msg = tail(stdout_lines);
    }
    if msg.is_empty() {
        msg = format!("çıkış kodu {}", status.code().unwrap_or(- 1));
    }
    bail!("{what} başarısız: {msg}")
}
fn remotes(root: &Path) -> Result<Vec<String>> {
    let t = git_text(root, &["remote"], "Uzak depolar okunamadı")?;
    Ok(t.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
}
fn require_remote(root: &Path) -> Result<Vec<String>> {
    let r = remotes(root)?;
    if r.is_empty() {
        bail!("Uzak depo tanımlı değil");
    }
    Ok(r)
}
fn current_branch(root: &Path) -> Option<String> {
    let out = run_git(root, &["symbolic-ref", "--short", "-q", "HEAD"]).ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}
fn upstream_of(root: &Path, branch: &str) -> Option<String> {
    let out = run_git(
            root,
            &[
                "for-each-ref",
                "--format=%(upstream:short)",
                &format!("refs/heads/{branch}"),
            ],
        )
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}
/// `git fetch --all --prune` with live progress lines.
pub fn fetch(root: &Path, on_progress: &mut dyn FnMut(&str)) -> Result<()> {
    require_remote(root)?;
    let mut c = git_cmd(root);
    c.args(["fetch", "--all", "--prune", "--progress"]);
    run_streaming(c, "git fetch", on_progress)
}
/// `git pull --no-edit` (merge strategy unless the user configured `pull.rebase`).
pub fn pull(root: &Path, on_progress: &mut dyn FnMut(&str)) -> Result<()> {
    require_remote(root)?;
    let Some(branch) = current_branch(root) else {
        bail!("Ayrık HEAD durumunda çekme yapılamaz");
    };
    if upstream_of(root, &branch).is_none() {
        bail!("'{branch}' dalının izlediği bir uzak dal yok");
    }
    let mut c = git_cmd(root);
    let has_rebase_cfg = run_git(root, &["config", "--get", "pull.rebase"])
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_rebase_cfg {
        c.args(["-c", "pull.rebase=false"]);
    }
    c.args(["pull", "--no-edit", "--progress"]);
    run_streaming(c, "git pull", on_progress)
}
/// `git push`; a branch without upstream is pushed with `-u <remote> <branch>`
/// (`origin` when present, otherwise the first remote).
pub fn push(root: &Path, on_progress: &mut dyn FnMut(&str)) -> Result<()> {
    let remotes = require_remote(root)?;
    let Some(branch) = current_branch(root) else {
        bail!("Ayrık HEAD durumunda gönderilemez");
    };
    let mut c = git_cmd(root);
    c.args(["push", "--progress"]);
    if upstream_of(root, &branch).is_none() {
        let remote = remotes.iter().find(|r| *r == "origin").unwrap_or(&remotes[0]);
        c.args(["-u", remote, &branch]);
    }
    run_streaming(c, "git push", on_progress)
}
/// Pull (when the branch has an upstream) then push.
pub fn sync(root: &Path, on_progress: &mut dyn FnMut(&str)) -> Result<()> {
    require_remote(root)?;
    if let Some(b) = current_branch(root) && upstream_of(root, &b).is_some() {
        pull(root, on_progress)?;
    }
    push(root, on_progress)
}
/// URL of `origin` (or the first remote); `None` when there is no remote.
pub fn remote_url(root: &Path) -> Option<String> {
    let r = remotes(root).ok()?;
    let name = r.iter().find(|n| *n == "origin").or_else(|| r.first())?;
    let out = run_git(root, &["remote", "get-url", name]).ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}
/// `git init` in `dir` (created when missing).
pub fn init(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| anyhow::Error::new(e).context("Klasör oluşturulamadı"))?;
    git_text(dir, &["init", "-q"], "Depo başlatılamadı").map(|_| ())
}
/// Clones `url` into a new folder under `dest_parent` (named after the repository) and returns
/// that folder.
pub fn clone(
    url: &str,
    dest_parent: &Path,
    on_progress: &mut dyn FnMut(&str),
) -> Result<PathBuf> {
    let url = url.trim();
    if url.is_empty() {
        bail!("Depo adresi boş");
    }
    let last = url
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or("");
    let name = last.strip_suffix(".git").unwrap_or(last);
    let name = if name.is_empty() { "repo" } else { name };
    std::fs::create_dir_all(dest_parent)
        .map_err(|e| anyhow::Error::new(e).context("Hedef klasör oluşturulamadı"))?;
    let mut c = git_cmd(dest_parent);
    c.args(["clone", "--progress", "--", url, name]);
    run_streaming(c, "git clone", on_progress)?;
    Ok(dest_parent.join(name))
}
/// Latest commit touching a path (or the whole repo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Abbreviated hash.
    pub sha: String,
    /// Subject line.
    pub message: String,
    pub author: String,
    /// Commit time, milliseconds since the Unix epoch.
    pub time_ms: i64,
}
/// `git log -1` for `path` (or HEAD when `None`). `None` when there is no such commit.
pub fn last_commit(root: &Path, path: Option<&Path>) -> Option<Commit> {
    let mut c = git_cmd(root);
    c.args(["log", "-1", "--format=%h%x00%s%x00%an%x00%ct"]);
    if let Some(p) = path {
        c.arg("--").arg(p);
    }
    let out = c.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let f: Vec<&str> = text.trim_end_matches(['\r', '\n']).split('\0').collect();
    if f.len() < 4 || f[0].is_empty() {
        return None;
    }
    Some(Commit {
        sha: f[0].into(),
        message: f[1].into(),
        author: f[2].into(),
        time_ms: f[3].trim().parse::<i64>().ok()?.saturating_mul(1000),
    })
}
/// The latest commit touching each of `names` (children of `dir`, a folder inside the
/// repository at `root`), like Files' commit columns. One `git log` streamed newest first and
/// stopped as soon as every name is found (or after `max_commits`): untracked names just stay
/// missing. Keys are the names as given.
pub fn last_commits_of_children(
    root: &Path,
    dir: &Path,
    names: &[String],
    max_commits: usize,
) -> HashMap<String, Commit> {
    use std::io::BufRead;
    let mut out = HashMap::new();
    let Some(rel) = rel_inside(root, dir) else { return out };
    let prefix = if rel.is_empty() { String::new() } else { format!("{rel}/") };
    let wanted: HashMap<String, &String> = names
        .iter()
        .map(|n| (n.to_lowercase(), n))
        .collect();
    if wanted.is_empty() {
        return out;
    }
    let mut c = git_cmd(root);
    c.args([
            "-c",
            "core.quotepath=false",
            "log",
            "--no-renames",
            "--name-only",
            "--format=%x01%h%x1f%s%x1f%an%x1f%ct",
            "--",
        ])
        .arg(if rel.is_empty() { ".".to_string() } else { rel.clone() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let Ok(mut child) = c.spawn() else { return out };
    let Some(stdout) = child.stdout.take() else { return out };
    let mut current: Option<Commit> = None;
    let mut commits = 0usize;
    for line in std::io::BufReader::new(stdout).split(b'\n') {
        let Ok(line) = line else { break };
        let line = String::from_utf8_lossy(&line);
        let line = line.trim_end_matches('\r');
        if let Some(h) = line.strip_prefix('\u{1}') {
            commits += 1;
            if commits > max_commits {
                break;
            }
            let f: Vec<&str> = h.split('\u{1f}').collect();
            current = (f.len() >= 4)
                .then(|| Commit {
                    sha: f[0].into(),
                    message: f[1].into(),
                    author: f[2].into(),
                    time_ms: f[3].trim().parse::<i64>().unwrap_or(0).saturating_mul(1000),
                });
            continue;
        }
        let (Some(cm), Some(rest)) = (&current, line.strip_prefix(prefix.as_str())) else {
            continue
        };
        let first = rest.split('/').next().unwrap_or("");
        if first.is_empty() {
            continue;
        }
        if let Some(name) = wanted.get(&first.to_lowercase()) {
            out.entry((*name).clone()).or_insert_with(|| cm.clone());
            if out.len() == wanted.len() {
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    out
}
/// `dir` relative to `root` with `/` separators ("" for the root itself); `None` outside it.
/// The root prefix matches case-insensitively (drive letters, Explorer-typed paths).
fn rel_inside(root: &Path, dir: &Path) -> Option<String> {
    let r = root.to_string_lossy().replace('\\', "/");
    let d = dir.to_string_lossy().replace('\\', "/");
    let (r, d) = (r.trim_end_matches('/'), d.trim_end_matches('/'));
    if d.len() < r.len() || !d.is_char_boundary(r.len())
        || !d[..r.len()].eq_ignore_ascii_case(r)
    {
        return None;
    }
    let rest = &d[r.len()..];
    if rest.is_empty() {
        return Some(String::new());
    }
    rest.strip_prefix('/').map(str::to_string)
}

/// Points `origin` at `url` (adds it, or changes its address).
pub fn connect_remote(root: &Path, url: &str) -> Result<()> {
    let url = url.trim();
    if url.is_empty() {
        bail!("Depo adresi boş");
    }
    let verb = if remotes(root)?.iter().any(|r| r == "origin") { "set-url" } else { "add" };
    git_text(root, &["remote", verb, "origin", url], "Uzak depo bağlanamadı").map(|_| ())
}

/// Whether there is anything to commit (changed, added, deleted or untracked files).
pub fn has_changes(root: &Path) -> bool {
    git_text(root, &["status", "--porcelain"], "").is_ok_and(|t| !t.trim().is_empty())
}

/// Stages everything and commits it with `message`.
pub fn commit_all(root: &Path, message: &str) -> Result<()> {
    let message = message.trim();
    if message.is_empty() {
        bail!("Kayıt açıklaması boş");
    }
    if !has_changes(root) {
        bail!("Kaydedilecek değişiklik yok");
    }
    git_text(root, &["add", "-A"], "Değişiklikler eklenemedi")?;
    git_text(root, &["commit", "-q", "-m", message], "Kaydedilemedi").map(|_| ())
}
