//! Installed apps for the quick search: name, an id to start it with. Linux: the desktop
//! entries (`.desktop` files) the app menus show, read by [`crate::xdg_apps`]; the id is the
//! desktop file ID.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub name: String,
    /// Desktop file ID (`org.kde.haruna.desktop`).
    pub id: String,
}

impl App {
    /// What draws its icon (rf-win: the shell's parsing name). Linux: the `.desktop` file's
    /// path, whose `Icon=` the thumbnailer draws (the ID itself when it isn't installed).
    pub fn parsing(&self) -> String {
        crate::xdg_apps::by_id(&self.id).map_or_else(|| self.id.clone(), |a| a.path.to_string_lossy().into_owned())
    }
}

/// Every app the menus show, sorted by name.
pub fn list() -> anyhow::Result<Vec<App>> {
    Ok(crate::xdg_apps::launchable().into_iter().map(|a| App { name: a.name.clone(), id: a.id.clone() }).collect())
}

/// Starts the app (no files).
pub fn launch(app: &App) -> anyhow::Result<()> {
    let a = crate::xdg_apps::by_id(&app.id).ok_or_else(|| anyhow::anyhow!("{}: not installed any more", app.name))?;
    crate::xdg_apps::launch(a, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever is installed: names are set, sorted, and every id finds its app again.
    #[test]
    fn apps_are_listed_by_name() {
        let l = list().unwrap();
        assert!(l.iter().all(|a| !a.name.is_empty() && a.id.ends_with(".desktop")));
        assert!(l.windows(2).all(|w| w[0].name.to_lowercase() <= w[1].name.to_lowercase()));
        assert!(l.iter().all(|a| crate::xdg_apps::by_id(&a.id).is_some()));
        // the icon comes from the .desktop file itself
        assert!(l.iter().all(|a| std::path::Path::new(&a.parsing()).is_file()));
    }
}
