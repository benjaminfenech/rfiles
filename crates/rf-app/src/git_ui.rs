//! Built-in Git (like Files): the status bar shows the branch and ahead/behind; the sync
//! button offers Pull / Push / Sync / Fetch, the branch button switches or creates branches.
//! Network operations run in the Status Center with git's progress lines.

use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName as L;
use gpui_kit::*;
use rf_linux::git::{self, CheckoutMode};

use crate::fluent_menu::{self, Item, Menu, MenuIcon};
use crate::folder_view::FolderView;
use crate::prompt::{self, Button, Kind};
use crate::t;

type View = WeakEntity<FolderView>;

fn refresh(view: &View, cx: &mut App) {
    let _ = view.update(cx, |v, cx| v.refresh(cx));
}

/// Pull / Push / Sync / Fetch menu.
pub fn sync_menu(view: View, root: PathBuf, ahead: u32, behind: u32, pos: Point<Pixels>, window: &mut Window, cx: &mut App) {
    type Op = fn(&Path, &mut dyn FnMut(&str)) -> anyhow::Result<()>;
    let item = |label: &str, icon: L, hint: Option<String>, title: &'static str, op: Op| {
        let (view, root) = (view.clone(), root.clone());
        let mut it = Item::action(label.to_string(), MenuIcon::Lucide(icon), move |_, cx| run_op(cx, format!("{}: {}", crate::i18n::t(title), repo_name(&root)), root.clone(), op, view.clone()));
        if let Some(h) = hint {
            it = it.hint(h);
        }
        it
    };
    let remote = git::remote_url(&root);
    let mut items = Vec::new();
    if remote.is_some() {
        items.extend([
            item(t!("Pull"), L::ArrowDown, (behind > 0).then(|| format!("↓{behind}")), "Pulling", git::pull),
            item(t!("Push"), L::ArrowUp, (ahead > 0).then(|| format!("↑{ahead}")), "Pushing", git::push),
            item(t!("Sync"), L::RefreshCw, None, "Syncing", git::sync),
            Item::Separator,
            item(t!("Fetch"), L::CloudDownload, None, "Fetching", git::fetch),
            Item::Separator,
        ]);
    }
    {
        let (view, root) = (view.clone(), root.clone());
        items.push(Item::action(t!("Commit changes…"), MenuIcon::Lucide(L::GitCommitHorizontal), move |window, cx| commit_dialog(view.clone(), root.clone(), window, cx)));
    }
    {
        let (view, root, current) = (view.clone(), root.clone(), remote.clone());
        let label = if remote.is_some() { t!("Change remote address…") } else { t!("Connect to a remote…") };
        items.push(Item::action(label, MenuIcon::Lucide(L::Link), move |window, cx| connect_dialog(view.clone(), root.clone(), current.clone(), window, cx)));
    }
    fluent_menu::show(Menu { quick: Vec::new(), items }, pos, window, cx);
}

fn repo_name(root: &Path) -> String {
    rf_core::history::display_name(root)
}

fn run_op(cx: &mut App, title: String, root: PathBuf, op: fn(&Path, &mut dyn FnMut(&str)) -> anyhow::Result<()>, view: View) {
    crate::status_center::start_with(
        cx,
        title,
        move |t| {
            op(&root, &mut |line: &str| (t.on_progress)(0, 0, Some(line.to_string())))?;
            Ok(true)
        },
        Some(Box::new(move |_, cx| refresh(&view, cx))),
    );
}

/// "Commit changes…": a message, then everything is staged and committed.
fn commit_dialog(view: View, root: PathBuf, window: &mut Window, cx: &mut App) {
    let go = Button::new(t!("Save"), Kind::Primary, move |msg, _, cx| {
        let msg = msg.unwrap_or_default().trim().to_string();
        if msg.is_empty() {
            return Err(t!("Describe the change in a few words").into());
        }
        let (root, view) = (root.clone(), view.clone());
        let title = format!("{} {}", t!("Committing:"), repo_name(&root));
        crate::status_center::start_with(
            cx,
            title,
            move |_| {
                git::commit_all(&root, &msg)?;
                Ok(true)
            },
            Some(Box::new(move |_, cx| refresh(&view, cx))),
        );
        Ok(())
    });
    prompt::show(
        t!("Commit changes"),
        Some(t!("Every change in this repository (new files included) is saved as one commit.").to_string()),
        Some((String::new(), t!("For example: Updated the cover image").to_string())),
        vec![go, Button::cancel()],
        window,
        cx,
    );
}

/// "Connect to a remote…" / "Change remote address…": sets `origin`; "Connect and push" also
/// sends the current branch (with upstream) right away.
fn connect_dialog(view: View, root: PathBuf, current: Option<String>, window: &mut Window, cx: &mut App) {
    let start = |push: bool, view: View, root: PathBuf| {
        move |url: Option<String>, _: &mut Window, cx: &mut App| -> Result<(), String> {
            let url = url.unwrap_or_default().trim().to_string();
            if url.is_empty() {
                return Err(t!("Enter the repository address").into());
            }
            let (root, view) = (root.clone(), view.clone());
            let title = format!("{} {}", t!("Connecting to remote:"), repo_name(&root));
            crate::status_center::start_with(
                cx,
                title,
                move |t| {
                    git::connect_remote(&root, &url)?;
                    if push {
                        git::push(&root, &mut |line: &str| (t.on_progress)(0, 0, Some(line.to_string())))?;
                    }
                    Ok(true)
                },
                Some(Box::new(move |_, cx| refresh(&view, cx))),
            );
            Ok(())
        }
    };
    let mut buttons = Vec::new();
    if current.is_none() {
        buttons.push(Button::new(t!("Connect and push"), Kind::Primary, start(true, view.clone(), root.clone())));
        buttons.push(Button::new(t!("Connect only"), Kind::Normal, start(false, view.clone(), root.clone())));
    } else {
        buttons.push(Button::new(t!("Replace"), Kind::Primary, start(false, view.clone(), root.clone())));
    }
    buttons.push(Button::cancel());
    prompt::show(
        if current.is_none() { t!("Connect to a remote") } else { t!("Remote address") },
        Some(t!("The address of an empty repository on GitHub, GitLab or another server. Create the repository there first.").to_string()),
        Some((current.unwrap_or_default(), "https://github.com/kullanici/depo.git".to_string())),
        buttons,
        window,
        cx,
    );
}

/// "Initialize Git repository" for a folder that is not in one.
pub fn init_repo(view: View, dir: PathBuf, cx: &mut App) {
    let title = format!("{} {}", t!("Initializing Git repository:"), repo_name(&dir));
    crate::status_center::start_with(
        cx,
        title,
        move |_| {
            git::init(&dir)?;
            Ok(true)
        },
        Some(Box::new(move |_, cx| refresh(&view, cx))),
    );
}

/// "Clone repository…": asks for the address, clones into a new folder inside `dir`.
pub fn clone_dialog(view: View, dir: PathBuf, window: &mut Window, cx: &mut App) {
    let go = Button::new(t!("Clone"), Kind::Primary, move |url, _, cx| {
        let url = url.unwrap_or_default().trim().to_string();
        if url.is_empty() {
            return Err(t!("Enter the repository address").into());
        }
        let (dir, view) = (dir.clone(), view.clone());
        let title = format!("{} {}", t!("Cloning:"), url.rsplit('/').next().unwrap_or(&url).trim_end_matches(".git"));
        crate::status_center::start_with(
            cx,
            title,
            move |t| {
                git::clone(&url, &dir, &mut |line: &str| (t.on_progress)(0, 0, Some(line.to_string())))?;
                Ok(true)
            },
            Some(Box::new(move |_, cx| {
                if let Some(v) = view.upgrade() {
                    v.update(cx, |v, cx| v.refresh(cx));
                }
            })),
        );
        Ok(())
    });
    prompt::show(
        t!("Clone repository"),
        Some(t!("The repository is cloned into a new folder inside this one.").to_string()),
        Some((String::new(), "https://github.com/kullanici/depo.git".to_string())),
        vec![go, Button::cancel()],
        window,
        cx,
    );
}

/// Branch list (local, then remote) with "New branch…"; picking one switches to it.
pub fn branch_menu(view: View, root: PathBuf, pos: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let handle = window.window_handle();
    let task = cx.background_executor().spawn({
        let root = root.clone();
        async move { git::branches(&root) }
    });
    cx.spawn(async move |cx| {
        let branches = match task.await {
            Ok(b) => b,
            Err(e) => {
                log::warn!("git branches: {e:#}");
                return;
            }
        };
        let _ = cx.update_window(handle, |_, window, cx| {
            let mut items = Vec::new();
            {
                let (view, root) = (view.clone(), root.clone());
                items.push(Item::action(t!("Create branch…"), MenuIcon::Lucide(L::GitBranchPlus), move |window, cx| {
                    new_branch_dialog(view.clone(), root.clone(), window, cx)
                }));
            }
            items.push(Item::Separator);
            let mut remote = Vec::new();
            let mut deletable = Vec::new();
            for b in branches {
                let hint = match (b.ahead, b.behind) {
                    (0, 0) => None,
                    (a, 0) => Some(format!("↑{a}")),
                    (0, d) => Some(format!("↓{d}")),
                    (a, d) => Some(format!("↑{a} ↓{d}")),
                };
                let (dview, droot) = (view.clone(), root.clone());
                let (view, root, name) = (view.clone(), root.clone(), b.name.clone());
                let is_head = b.is_head;
                let mut it = Item::action(b.name.clone(), MenuIcon::Lucide(L::GitBranch), move |window, cx| {
                    if !is_head {
                        checkout(view.clone(), root.clone(), name.clone(), CheckoutMode::Normal, window, cx)
                    }
                })
                .checked(b.is_head);
                if let Some(h) = hint {
                    it = it.hint(h);
                }
                if b.is_remote {
                    remote.push(it);
                } else {
                    items.push(it);
                    // (git refuses to delete the branch you are on)
                    if !b.is_head {
                        let (view, root, name) = (dview, droot, b.name.clone());
                        deletable.push(Item::action(b.name.clone(), MenuIcon::Lucide(L::GitBranch), move |window, cx| {
                            delete_branch_dialog(view.clone(), root.clone(), name.clone(), false, window, cx)
                        }));
                    }
                }
            }
            if !deletable.is_empty() {
                items.push(Item::Separator);
                items.push(Item::sub(t!("Delete branch"), MenuIcon::Lucide(L::Trash), deletable));
            }
            if !remote.is_empty() {
                items.push(Item::Separator);
                items.push(Item::sub(t!("Remote branches"), MenuIcon::Lucide(L::Cloud), remote));
            }
            fluent_menu::show(Menu { quick: Vec::new(), items }, pos, window, cx);
        });
    })
    .detach();
}

fn checkout(view: View, root: PathBuf, branch: String, mode: CheckoutMode, window: &mut Window, cx: &mut App) {
    let handle = window.window_handle();
    let task = cx.background_executor().spawn({
        let (root, branch) = (root.clone(), branch.clone());
        async move { git::checkout(&root, &branch, mode).map_err(|e| format!("{e:#}")) }
    });
    cx.spawn(async move |cx| {
        let r = task.await;
        let _ = cx.update_window(handle, |_, window, cx| match r {
            Ok(()) => refresh(&view, cx),
            Err(e) if e.starts_with("Couldn't switch branch: local changes") && matches!(mode, CheckoutMode::Normal) => {
                // Files asks what to do with uncommitted changes
                let (v1, r1, b1) = (view.clone(), root.clone(), branch.clone());
                let (v2, r2, b2) = (view.clone(), root.clone(), branch.clone());
                prompt::show(
                    t!("You have uncommitted changes"),
                    Some(format!("{} \"{branch}\"", t!("What should happen to your changes? Switching to"))),
                    None,
                    vec![
                        Button::new(t!("Bring them along"), Kind::Primary, move |_, window, cx| {
                            checkout(v1.clone(), r1.clone(), b1.clone(), CheckoutMode::Stash, window, cx);
                            Ok(())
                        }),
                        Button::new(t!("Discard changes"), Kind::Danger, move |_, window, cx| {
                            checkout(v2.clone(), r2.clone(), b2.clone(), CheckoutMode::Discard, window, cx);
                            Ok(())
                        }),
                        Button::cancel(),
                    ],
                    window,
                    cx,
                );
            }
            Err(e) => error_dialog(t!("Couldn't switch branch"), e, window, cx),
        });
    })
    .detach();
}

/// Asks, then deletes a local branch. A branch with commits that are in no other branch is
/// refused by git: the question is asked again (`force`), saying so.
fn delete_branch_dialog(view: View, root: PathBuf, branch: String, force: bool, window: &mut Window, cx: &mut App) {
    let msg = if force {
        format!("\"{branch}\" {}", t!("is not merged into another branch: if it is deleted, its commits can no longer be reached from a branch."))
    } else {
        format!("\"{branch}\" {}", t!("will be deleted as a local branch. The branch on the remote stays as it is."))
    };
    let label = if force { t!("Delete anyway") } else { t!("Delete") };
    prompt::show(
        t!("Delete the branch?"),
        Some(msg),
        None,
        vec![
            Button::new(label, Kind::Danger, move |_, window, cx| {
                let handle = window.window_handle();
                let task = cx.background_executor().spawn({
                    let (root, branch) = (root.clone(), branch.clone());
                    async move { git::delete_branch(&root, &branch, force).map_err(|e| format!("{e:#}")) }
                });
                let (view, root, branch) = (view.clone(), root.clone(), branch.clone());
                cx.spawn(async move |cx| {
                    let r = task.await;
                    let _ = cx.update_window(handle, |_, window, cx| match r {
                        Ok(()) => refresh(&view, cx),
                        Err(e) if !force && e.contains("not fully merged") => delete_branch_dialog(view.clone(), root.clone(), branch.clone(), true, window, cx),
                        Err(e) => error_dialog(t!("Couldn't delete the branch"), e, window, cx),
                    });
                })
                .detach();
                Ok(())
            }),
            Button::cancel(),
        ],
        window,
        cx,
    );
}

fn error_dialog(title: &str, message: String, window: &mut Window, cx: &mut App) {
    prompt::show(title.to_string(), Some(crate::i18n::td(&message)), None, vec![Button::new(t!("OK"), Kind::Primary, |_, _, _| Ok(()))], window, cx);
}

fn new_branch_dialog(view: View, root: PathBuf, window: &mut Window, cx: &mut App) {
    prompt::show(
        t!("New branch"),
        Some(t!("The new branch starts from the current one and is checked out.").to_string()),
        Some((String::new(), t!("Branch name").to_string())),
        vec![
            Button::new(t!("Create"), Kind::Primary, move |value, window, cx| {
                let name = value.unwrap_or_default().trim().to_string();
                if !git::is_valid_branch_name(&name) {
                    return Err(t!("Invalid branch name (no spaces, .., ~ ^ : ? * [ \\)").to_string());
                }
                let (view, root) = (view.clone(), root.clone());
                let handle = window.window_handle();
                let task = cx.background_executor().spawn({
                    let (root, name) = (root.clone(), name.clone());
                    async move { git::create_branch(&root, &name, None, true).map_err(|e| format!("{e:#}")) }
                });
                cx.spawn(async move |cx| {
                    let r = task.await;
                    let _ = cx.update_window(handle, |_, window, cx| match r {
                        Ok(()) => refresh(&view, cx),
                        Err(e) => error_dialog(t!("Couldn't create branch"), e, window, cx),
                    });
                })
                .detach();
                Ok(())
            }),
            Button::cancel(),
        ],
        window,
        cx,
    );
}
