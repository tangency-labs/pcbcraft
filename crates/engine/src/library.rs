//! Native library discovery and cached read queries, separate from document mutation.
use pcbcraft_library::{FootprintDefinition, SymbolDefinition};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const MAX_LIBRARY_BYTES: u64 = 32 * 1024 * 1024;
fn regular_metadata(path: &Path) -> Result<std::fs::Metadata, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "{}: library source must be a regular file",
            path.display()
        ));
    }
    if metadata.len() > MAX_LIBRARY_BYTES {
        return Err("library exceeds 32 MiB".into());
    }
    Ok(metadata)
}
fn read_library(path: &Path) -> Result<String, String> {
    regular_metadata(path)?;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("library source must be a regular file".into());
    }
    let mut text = String::new();
    file.take(MAX_LIBRARY_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if text.len() as u64 > MAX_LIBRARY_BYTES {
        return Err("library exceeds 32 MiB".into());
    }
    Ok(text)
}
fn scan_directory(path: &Path, limit: usize) -> Result<(Vec<PathBuf>, bool), String> {
    let mut directory = std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut files = Vec::new();
    for entry in directory.by_ref().take(limit) {
        files.push(
            entry
                .map_err(|e| format!("{}: {e}", path.display()))?
                .path(),
        );
    }
    let truncated = directory
        .next()
        .transpose()
        .map_err(|e| format!("{}: {e}", path.display()))?
        .is_some();
    Ok((files, truncated))
}

struct CachedSymbols {
    path: PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
    definitions: Arc<Vec<SymbolDefinition>>,
}
// Metadata-based cache: rewriting a file while preserving both length and mtime
// can retain a stale entry. This is a performance cache, not content identity.
// Files without usable modification times are always reparsed. Restart the
// session to refresh deliberately timestamp-preserving external replacements.
static CACHE: OnceLock<Mutex<Vec<CachedSymbols>>> = OnceLock::new();
pub fn symbols(path: &Path) -> Result<Arc<Vec<SymbolDefinition>>, String> {
    if path.extension().and_then(|s| s.to_str()) != Some("kicad_sym") {
        return Err("select a .kicad_sym symbol library file".into());
    }
    let metadata = regular_metadata(path)?;
    let modified = metadata.modified().ok();
    let cache = CACHE.get_or_init(|| Mutex::new(vec![]));
    if let Some(definitions) = cache
        .lock()
        .map_err(|_| "library cache unavailable")?
        .iter()
        .find(|c| {
            modified.is_some()
                && c.path == path
                && c.len == metadata.len()
                && c.modified == modified
        })
        .map(|c| c.definitions.clone())
    {
        return Ok(definitions);
    }
    let text = read_library(path)?;
    let definitions = Arc::new(pcbcraft_library::read_symbols(&text).map_err(|e| e.to_string())?);
    let mut cache = cache.lock().map_err(|_| "library cache unavailable")?;
    cache.retain(|c| c.path != path);
    while cache.len() >= 2 {
        cache.remove(0);
    }
    cache.push(CachedSymbols {
        path: path.into(),
        len: metadata.len(),
        modified,
        definitions: definitions.clone(),
    });
    Ok(definitions)
}
pub fn symbol(path: &Path, name: &str) -> Result<SymbolDefinition, String> {
    symbols(path)?
        .iter()
        .find(|s| s.name == name)
        .cloned()
        .ok_or_else(|| format!("symbol {name} not found in {}", path.display()))
}
pub fn footprint(path: &Path) -> Result<FootprintDefinition, String> {
    if path.extension().and_then(|s| s.to_str()) != Some("kicad_mod") {
        return Err("select a .kicad_mod footprint file".into());
    }
    pcbcraft_library::read_footprint(&read_library(path)?).map_err(|e| e.to_string())
}
fn paths() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/usr/share/kicad"),
        PathBuf::from("/usr/local/share/kicad"),
        PathBuf::from("C:/Program Files/KiCad/10.0/share/kicad"),
        PathBuf::from("C:/Program Files/KiCad/9.0/share/kicad"),
        PathBuf::from(r"\\wsl.localhost\Ubuntu\usr\share\kicad"),
    ];
    for version in ["10", "9", "8"] {
        for suffix in ["SYMBOL_DIR", "FOOTPRINT_DIR"] {
            if let Ok(path) = std::env::var(format!("KICAD{version}_{suffix}")) {
                roots.push(PathBuf::from(path));
            }
        }
    }
    roots
}
pub fn catalog() -> Result<Value, String> {
    catalog_paths(paths(), 4096)
}
fn catalog_paths(roots: Vec<PathBuf>, scan_limit: usize) -> Result<Value, String> {
    let mut symbol_files = vec![];
    let mut footprint_libraries = vec![];
    let mut truncated_directories = vec![];
    let mut seen = std::collections::BTreeSet::new();
    for root in roots {
        for (directory, symbols) in [
            (root.join("symbols"), true),
            (root.join("footprints"), false),
            (root.clone(), true),
            (root, false),
        ] {
            if !seen.insert((directory.clone(), symbols)) {
                continue;
            }
            match std::fs::metadata(&directory) {
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("{}: {e}", directory.display())),
            }
            let (entries, truncated) = scan_directory(&directory, scan_limit)?;
            if truncated {
                truncated_directories.push(directory.to_string_lossy().into_owned());
            }
            for path in entries {
                let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                let item = json!({"name":path.file_stem().and_then(|s|s.to_str()).unwrap_or(""),"path":path.to_string_lossy()});
                if symbols
                    && extension == "kicad_sym"
                    && std::fs::metadata(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))?
                        .is_file()
                {
                    symbol_files.push(item);
                } else if !symbols
                    && extension == "pretty"
                    && std::fs::metadata(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))?
                        .is_dir()
                {
                    footprint_libraries.push(item);
                }
            }
        }
    }
    symbol_files.sort_by(|a, b| {
        a.get("name")
            .and_then(Value::as_str)
            .cmp(&b.get("name").and_then(Value::as_str))
    });
    footprint_libraries.sort_by(|a, b| {
        a.get("name")
            .and_then(Value::as_str)
            .cmp(&b.get("name").and_then(Value::as_str))
    });
    symbol_files.dedup();
    footprint_libraries.dedup();
    truncated_directories.sort();
    truncated_directories.dedup();
    Ok(
        json!({"symbol_libraries":symbol_files,"footprint_libraries":footprint_libraries,"scan_truncated":!truncated_directories.is_empty(),"truncated_directories":truncated_directories,"license":"KiCad official libraries: CC-BY-SA-4.0 with design exception; third-party libraries retain their own licenses","license_url":"https://www.kicad.org/libraries/license/"}),
    )
}
pub fn search(path: &Path, kind: &str, query: &str, limit: usize) -> Result<Value, String> {
    if query.len() > 256 || limit == 0 || limit > 100 {
        return Err("query must be <=256 bytes and limit 1–100".into());
    }
    let query = query.to_lowercase();
    let mut entries = vec![];
    let mut total = 0;
    let mut scan_truncated = false;
    match kind {
        "symbol" => {
            let definitions = symbols(path)?;
            for s in definitions.iter().filter(|s| {
                format!("{} {} {}", s.name, s.description, s.keywords)
                    .to_lowercase()
                    .contains(&query)
            }) {
                total += 1;
                if entries.len() < limit {
                    entries.push(json!({"name":s.name,"description":s.description,"keywords":s.keywords,"footprint":s.footprint,"pins":s.pins.len(),"units":s.pins.iter().map(|p|p.unit).max().unwrap_or(1),"path":path.to_string_lossy(),"kind":"symbol","unsupported":s.unsupported}));
                }
            }
        }
        "footprint" => {
            let (directory, truncated) = scan_directory(path, 10_000)?;
            scan_truncated = truncated;
            let mut files = vec![];
            for path in directory {
                if path.extension().and_then(|s| s.to_str()) == Some("kicad_mod")
                    && std::fs::metadata(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))?
                        .is_file()
                {
                    files.push(path);
                }
            }
            files.sort();
            for file in files {
                let name = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !name.to_lowercase().contains(&query) {
                    continue;
                }
                total += 1;
                if entries.len() < limit {
                    entries.push(
                        json!({"name":name,"path":file.to_string_lossy(),"kind":"footprint"}),
                    );
                }
            }
        }
        _ => return Err("kind must be symbol or footprint".into()),
    }
    Ok(
        json!({"entries":entries,"total":total,"limit":limit,"truncated":total>limit||scan_truncated,"scan_truncated":scan_truncated}),
    )
}
pub fn read_tool(name: &str, args: &Value) -> Result<Value, String> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing {key}"))
    };
    match name {
        "library.catalog" => catalog(),
        "library.search" => {
            let limit = match args.get("limit") {
                None => 30,
                Some(v) => v
                    .as_u64()
                    .filter(|v| (1..=100).contains(v))
                    .ok_or("limit must be an integer from 1 to 100")?,
            };
            let limit = usize::try_from(limit).map_err(|_| "limit exceeds platform bounds")?;
            let query = match args.get("query") {
                None => "",
                Some(v) => v.as_str().ok_or("query must be a string")?,
            };
            search(Path::new(text("path")?), text("kind")?, query, limit)
        }
        "library.inspect_symbol" => {
            let mut definition = symbol(Path::new(text("path")?), text("name")?)?;
            definition.raw_source.clear();
            Ok(json!({"definition":definition,"kind":"symbol","path":text("path")?}))
        }
        "library.inspect_footprint" => {
            let mut definition = footprint(Path::new(text("path")?))?;
            definition.raw_source.clear();
            for pad in &mut definition.pads {
                pad.raw_source.clear();
            }
            Ok(json!({"definition":definition,"kind":"footprint","path":text("path")?}))
        }
        _ => Err("unknown library query".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Result<Self, String> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "pcbcraft-library-test-{}-{timestamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).map_err(|e| e.to_string())?;
            Ok(Self(path))
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn library_search_rejects_bad_json_types_before_filesystem_access() {
        for limit in [
            json!(0),
            json!(101),
            json!(-1),
            json!(1.5),
            json!("10"),
            json!(null),
            json!(4_294_967_297u64),
        ] {
            let args = json!({"path":"missing.kicad_sym","kind":"symbol","limit":limit});
            assert!(read_tool("library.search", &args).is_err_and(|e| e.contains("limit")));
        }
        assert!(
            read_tool(
                "library.search",
                &json!({"path":"missing.kicad_sym","kind":"symbol","query":true})
            )
            .is_err_and(|e| e.contains("query"))
        );
    }
    #[test]
    fn library_reads_are_regular_bounded_and_independent_of_project_cap() -> Result<(), String> {
        let directory = TestDirectory::new()?;
        let path = directory.0.join("large.kicad_sym");
        let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        file.write_all(&vec![b' '; 8 * 1024 * 1024 + 128])
            .map_err(|e| e.to_string())?;
        file.write_all(b"(kicad_symbol_lib (symbol X))")
            .map_err(|e| e.to_string())?;
        drop(file);
        assert_eq!(symbols(&path)?.len(), 1);
        assert!(read_library(&directory.0).is_err_and(|e| e.contains("regular file")));
        let too_large = directory.0.join("oversized.kicad_sym");
        std::fs::File::create(&too_large)
            .and_then(|f| f.set_len(MAX_LIBRARY_BYTES + 1))
            .map_err(|e| e.to_string())?;
        assert!(symbols(&too_large).is_err_and(|e| e.contains("32 MiB")));
        let non_file = directory.0.join("directory.kicad_mod");
        std::fs::create_dir(&non_file).map_err(|e| e.to_string())?;
        assert!(footprint(&non_file).is_err_and(|e| e.contains("regular file")));
        Ok(())
    }
    #[test]
    fn directory_scans_report_truncation_and_errors() -> Result<(), String> {
        let directory = TestDirectory::new()?;
        for name in ["A.kicad_sym", "B.kicad_sym"] {
            std::fs::write(directory.0.join(name), "(kicad_symbol_lib)")
                .map_err(|e| e.to_string())?;
        }
        let (files, truncated) = scan_directory(&directory.0, 1)?;
        assert_eq!(files.len(), 1);
        assert!(truncated);
        let (_, truncated) = scan_directory(&directory.0, 2)?;
        assert!(!truncated);
        assert!(scan_directory(&directory.0.join("absent"), 10).is_err());
        let catalog = catalog_paths(vec![directory.0.clone()], 1)?;
        assert_eq!(catalog["scan_truncated"], true);
        assert_eq!(
            catalog["truncated_directories"].as_array().map(Vec::len),
            Some(1)
        );
        Ok(())
    }
    #[test]
    fn footprint_search_only_returns_files_and_reports_result_limit() -> Result<(), String> {
        let directory = TestDirectory::new()?;
        for name in ["A.kicad_mod", "B.kicad_mod"] {
            std::fs::write(directory.0.join(name), "(footprint X)").map_err(|e| e.to_string())?;
        }
        std::fs::create_dir(directory.0.join("C.kicad_mod")).map_err(|e| e.to_string())?;
        let results = search(&directory.0, "footprint", "", 1)?;
        assert_eq!(results["total"], 2);
        assert_eq!(results["truncated"], true);
        assert_eq!(results["scan_truncated"], false);
        assert_eq!(results["entries"].as_array().map(Vec::len), Some(1));
        Ok(())
    }
}
