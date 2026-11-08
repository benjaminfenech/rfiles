//! XDG user directories (`~/.config/user-dirs.dirs`, through the `dirs` crate).

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownFolder {
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
    Profile,
}

impl KnownFolder {
    pub const PINNED_DEFAULTS: [KnownFolder; 6] = [
        KnownFolder::Desktop,
        KnownFolder::Downloads,
        KnownFolder::Documents,
        KnownFolder::Pictures,
        KnownFolder::Music,
        KnownFolder::Videos,
    ];

    pub fn title(self) -> &'static str {
        match self {
            KnownFolder::Desktop => "Masaüstü",
            KnownFolder::Downloads => "İndirilenler",
            KnownFolder::Documents => "Belgeler",
            KnownFolder::Pictures => "Resimler",
            KnownFolder::Music => "Müzik",
            KnownFolder::Videos => "Videolar",
            KnownFolder::Profile => "Kullanıcı",
        }
    }

    /// The folder, when it is set up and exists. An unset XDG folder falls back to home in
    /// `dirs`; that one is skipped so home doesn't show up six times.
    pub fn path(self) -> Option<PathBuf> {
        let home = dirs::home_dir();
        let p = match self {
            KnownFolder::Desktop => dirs::desktop_dir(),
            KnownFolder::Downloads => dirs::download_dir(),
            KnownFolder::Documents => dirs::document_dir(),
            KnownFolder::Pictures => dirs::picture_dir(),
            KnownFolder::Music => dirs::audio_dir(),
            KnownFolder::Videos => dirs::video_dir(),
            KnownFolder::Profile => return home,
        }?;
        (Some(&p) != home.as_ref() && p.is_dir()).then_some(p)
    }
}

pub fn known_folders() -> Vec<(KnownFolder, PathBuf)> {
    KnownFolder::PINNED_DEFAULTS.iter().filter_map(|k| k.path().map(|p| (*k, p))).collect()
}
