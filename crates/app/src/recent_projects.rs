//! Desktop project history. Paths are recorded only after a file job succeeds, and live in
//! application storage rather than in the project or its electrical undo history.

use std::path::PathBuf;

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
#[derive(Debug, Default)]
pub(super) struct RecentProjects {
    entries: Vec<RecentProject>,
    changed: bool,
}

impl RecentProjects {
    /// Restore bounded, unique absolute paths without touching the filesystem. Bad settings
    /// fall back to an empty history; missing files remain visible until explicitly removed.
    pub(super) fn parse(text: &str) -> Self {
        let mut recent = Self::default();
        if text.len() > MAX_SETTINGS_BYTES {
            return recent;
        }
        if let Ok(entries) = serde_json::from_str::<Vec<RecentProject>>(text) {
            for entry in entries {
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
        self.changed = true;
    }

    /// Forget the entire history without deleting any projects.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.changed = true;
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
            recent.remember(&format!("Project {index}"), root.join(format!("{index}.pc.zip")));
        }
        assert_eq!(recent.entries().len(), MAX_RECENT);
        recent.remember("Renamed", root.join("15.pc.zip"));
        assert_eq!(recent.entries()[0].name, "Renamed");
        assert_eq!(recent.entries().len(), MAX_RECENT);
        let restored = RecentProjects::parse(&recent.to_text());
        assert_eq!(restored.entries(), recent.entries());
        assert!(!RecentProjects::parse(&recent.to_text()).take_changed());
        assert!(recent.take_changed());
        assert!(!recent.take_changed());
        recent.remove(&root.join("15.pc.zip"));
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
}
