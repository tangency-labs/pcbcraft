//! Native filesystem access, shared by desktop file workers and the headless CLI.

use pcbcraft_engine::{
    Project,
    persistence::{ENTRY_POINT, Encoding, decode_documents, decode_file, read_bytes},
};
use std::{
    fs::{self, File},
    path::Path,
    sync::Arc,
};

mod folder;
pub use folder::{load, save};

/// Open a folder, its project.pc entry point, a packed archive or a legacy single-file project.
/// Interrupted folder saves recover under the writer lock before any documents are adopted.
pub fn open(path: &Path) -> Result<Arc<Project>, String> {
    let name = path.file_name().ok_or("Project path has no filename")?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    let path = parent.join(name);
    if name == ENTRY_POINT && parent.join(".pcbcraft-save").try_exists().map_err(|e| e.to_string())? {
        return decode_documents(&load(&parent)?);
    }
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if metadata.is_dir() {
        return decode_documents(&load(&path)?);
    }
    if !metadata.is_file() {
        return Err("Project path is not a regular file or directory.".to_owned());
    }
    let bytes = read_bytes(File::open(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if Encoding::detect(&bytes) == Encoding::Json {
        let header: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if header["format"] == "pcbcraft-project/5" {
            if name != ENTRY_POINT {
                return Err("A split project's entry point must be named project.pc.".to_owned());
            }
            return decode_documents(&load(&parent)?);
        }
    }
    decode_file(&bytes).map(|(project, _)| project)
}
