//! Desktop project history. Paths are recorded only after a file job succeeds, and live in
//! application storage rather than in the project or its electrical undo history.

use std::path::PathBuf;

use crate::schematic_canvas::Thumbnail;
use pcbcraft_chrome::preview::PreviewCache;
use pcbcraft_storage::plain_path_if_equivalent;
use serde::{Deserialize, Serialize};

/// Match the family start page's bounded history, newest first.
const MAX_RECENT: usize = 12;
/// Bound settings parsing independently of the project loader.
const MAX_SETTINGS_BYTES: usize = 256 * 1024;

/// Display metadata and the native target from the last successful open or save.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RecentProject {
    /// Saved project name, refreshed when the target next opens or saves.
    pub name: String,
    /// Normalized absolute folder, archive or legacy file path.
    pub path: PathBuf,
}

/// Bounded application history, with a change flag for prompt persistence.
#[derive(Debug)]
pub(super) struct RecentProjects {
    entries: Vec<RecentProject>,
    changed: bool,
    previews: PreviewCache<PathBuf, Thumbnail>,
}

impl Default for RecentProjects {
    /// History and preview storage share the same bound; preview work never owns the editor.
    fn default() -> Self {
        Self { entries: Vec::new(), changed: false, previews: PreviewCache::new(MAX_RECENT) }
    }
}

impl RecentProjects {
    /// Restore bounded, unique absolute paths without touching the filesystem. Bad settings
    /// fall back to an empty history; missing files remain visible until explicitly removed.
    /// Windows `\\?\C:\…` paths saved by earlier builds are restored in their plain `C:\…` form
    /// when that names the same file; see [`plain_path_if_equivalent`].
    pub(super) fn parse(text: &str) -> Self {
        let mut recent = Self::default();
        if text.len() > MAX_SETTINGS_BYTES {
            return recent;
        }
        if let Ok(entries) = serde_json::from_str::<Vec<RecentProject>>(text) {
            for mut entry in entries {
                // The file worker now records plain paths (#154); restoring the same form keeps
                // one project from being listed twice.
                entry.path = plain_path_if_equivalent(entry.path);
                if entry.path.is_absolute() && !entry.name.trim().is_empty() && !recent.entries.iter().any(|old| old.path == entry.path) {
                    recent.entries.push(entry);
                    if recent.entries.len() == MAX_RECENT {
                        break;
                    }
                }
            }
        }
        recent
    }

    /// Encode history alone, leaving project files and other application settings unchanged.
    pub(super) fn to_text(&self) -> String {
        serde_json::to_string(&self.entries).unwrap_or_default()
    }

    /// Return the most recently opened or saved projects first.
    pub(super) fn entries(&self) -> &[RecentProject] {
        &self.entries
    }

    /// Promote a successful file target, replacing its stored name and removing duplicates.
    /// Paths come normalized from the file worker. Non-Unicode paths cannot round-trip through
    /// JSON settings and are omitted without affecting opening or saving the project itself.
    pub(super) fn remember(&mut self, name: &str, path: PathBuf) {
        if !path.is_absolute() || path.to_str().is_none() {
            return;
        }
        self.entries.retain(|entry| entry.path != path);
        self.entries.insert(0, RecentProject { name: name.to_owned(), path });
        self.entries.truncate(MAX_RECENT);
        self.changed = true;
    }

    /// Forget a path without deleting or otherwise accessing its project on disk.
    pub(super) fn remove(&mut self, path: &std::path::Path) {
        self.entries.retain(|entry| entry.path != path);
        self.retain_previews();
        self.changed = true;
    }

    /// Forget the entire history without deleting any projects.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.retain_previews();
        self.changed = true;
    }

    /// Cache a freshly opened/saved snapshot instead of rereading its file. Geometry preparation
    /// runs on the preview worker, and a pending older load cannot replace this saved version.
    pub(super) fn remember_project(&mut self, project: std::sync::Arc<pcbcraft_engine::Project>, path: PathBuf) {
        self.remember(&project.name, path.clone());
        self.retain_previews();
        self.previews.enqueue(path, move || Thumbnail::new(&project));
    }

    /// Read missing previews independently of Open/Save, without modifying the files or session.
    /// A failed preview stays terminal until that project is opened or saved successfully again.
    pub(super) fn prepare_previews(&mut self, ctx: &egui::Context) {
        self.retain_previews();
        for entry in &self.entries {
            if !self.previews.contains(&entry.path) {
                let path = entry.path.clone();
                self.previews.enqueue(path.clone(), move || pcbcraft_storage::preview(&path).ok().and_then(|project| Thumbnail::new(&project)));
            }
        }
        self.previews.poll(ctx);
    }

    /// Borrow a ready schematic, a terminal unavailable result, or a still-pending request.
    pub(super) fn preview(&self, path: &std::path::Path) -> Option<Option<&Thumbnail>> {
        self.previews.get(&path.to_owned())
    }

    /// Release queued work and cached geometry for forgotten paths, retaining running worker
    /// slots until their results arrive so repeated clearing cannot spawn unbounded threads.
    fn retain_previews(&mut self) {
        self.previews.retain(|path| self.entries.iter().any(|entry| &entry.path == path));
    }

    /// Consume the flag used to flush successful file operations and removals promptly.
    pub(super) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Promotion deduplicates and updates names; forgetting affects only application history.
    #[test]
    fn history_is_bounded_unique_and_round_trips() {
        let mut recent = RecentProjects::default();
        let root = std::env::temp_dir();
        for index in 0..20 {
            recent.remember(&format!("Project {index}"), root.join(format!("{index}.pc")));
        }
        assert_eq!(recent.entries().len(), MAX_RECENT);
        recent.remember("Renamed", root.join("15.pc"));
        assert_eq!(recent.entries()[0].name, "Renamed");
        assert_eq!(recent.entries().len(), MAX_RECENT);
        let restored = RecentProjects::parse(&recent.to_text());
        assert_eq!(restored.entries(), recent.entries());
        assert!(!RecentProjects::parse(&recent.to_text()).take_changed());
        assert!(recent.take_changed());
        assert!(!recent.take_changed());
        recent.remove(&root.join("15.pc"));
        assert!(recent.entries().iter().all(|entry| entry.name != "Renamed"));
        recent.clear();
        assert!(recent.entries().is_empty());
    }

    /// Invalid settings do not prevent startup, and stored relative paths are never reopened.
    #[test]
    fn invalid_settings_are_ignored_without_disk_access() {
        for text in ["broken", "{}", r#"[{"name":"Bad","path":"relative.pc"}]"#] {
            assert!(RecentProjects::parse(text).entries().is_empty());
        }
        assert!(RecentProjects::parse(&" ".repeat(MAX_SETTINGS_BYTES + 1)).entries().is_empty());
        let path = std::env::temp_dir().join("not-required-to-exist.pc");
        let text = serde_json::to_string(&vec![
            RecentProject { name: "First".to_owned(), path: path.clone() },
            RecentProject { name: "Duplicate".to_owned(), path },
        ])
        .unwrap();
        let restored = RecentProjects::parse(&text);
        assert_eq!(restored.entries().len(), 1);
        assert_eq!(restored.entries()[0].name, "First");
    }

    /// History saved before #154 holds Windows `\\?\C:\…` paths. Restoring shows the plain
    /// form and merges it with the same project's plain entry instead of listing it twice.
    #[cfg(windows)]
    #[test]
    fn windows_history_from_older_builds_uses_plain_drive_paths() {
        let plain = PathBuf::from(r"C:\Projects\board.pc");
        let text = serde_json::to_string(&vec![
            RecentProject { name: "Older build".to_owned(), path: PathBuf::from(r"\\?\C:\Projects\board.pc") },
            RecentProject { name: "Current build".to_owned(), path: plain.clone() },
        ])
        .unwrap();
        let restored = RecentProjects::parse(&text);
        assert_eq!(restored.entries(), [RecentProject { name: "Older build".to_owned(), path: plain }]);
    }

    /// A stored `\\?\` path stays as it is when the plain spelling would open something else.
    /// Guards the #155 review: every Windows version drops the trailing dot from
    /// `C:\Projects\board.pc.`, naming a different file.
    #[cfg(windows)]
    #[test]
    fn windows_history_keeps_verbatim_paths_whose_plain_form_names_another_file() {
        let verbatim = PathBuf::from(r"\\?\C:\Projects\board.pc.");
        let text = serde_json::to_string(&vec![RecentProject { name: "Trailing dot".to_owned(), path: verbatim.clone() }]).unwrap();
        assert_eq!(RecentProjects::parse(&text).entries()[0].path, verbatim);
    }

    /// A stored device-named path such as `\\?\C:\Projects\CON.pc` keeps `\\?\` exactly when
    /// this Windows reads the plain spelling as the device: Windows 10 and earlier do, Windows 11
    /// reads it as an ordinary file and restores the plain form.
    #[cfg(windows)]
    #[test]
    fn windows_history_follows_how_windows_reads_device_names() {
        let verbatim = PathBuf::from(r"\\?\C:\Projects\CON.pc");
        let plain = PathBuf::from(r"C:\Projects\CON.pc");
        let expected = if std::path::absolute(&plain).unwrap() == plain { plain } else { verbatim.clone() };
        let text = serde_json::to_string(&vec![RecentProject { name: "Device name".to_owned(), path: verbatim }]).unwrap();
        assert_eq!(RecentProjects::parse(&text).entries()[0].path, expected);
    }
}
