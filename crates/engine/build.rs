//! Embed native library documents for desktop and browser through the same source adapter.
use std::{env, error::Error, fs, path::Path};

/// Discover bundled library folders and item documents deterministically; invalid I/O fails
/// the build. Native schema and identity validation remains in the shared runtime loader.
fn main() -> Result<(), Box<dyn Error>> {
    let manifest = env::var_os("CARGO_MANIFEST_DIR").ok_or("Cargo did not provide CARGO_MANIFEST_DIR")?;
    let root = Path::new(&manifest).join("../../libraries");
    println!("cargo:rerun-if-changed={}", root.display());
    let selection = root.join("bundled.txt");
    println!("cargo:rerun-if-changed={}", selection.display());
    let mut libraries = Vec::new();
    for id in fs::read_to_string(selection)?.lines().map(str::trim).filter(|id| !id.is_empty()) {
        if !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            return Err("Invalid bundled library folder name".into());
        }
        let path = root.join(id);
        if libraries.contains(&path) {
            return Err("Duplicate bundled library folder".into());
        }
        libraries.push(path);
    }
    libraries.sort();
    let mut code = String::from("pub(crate) const LIBRARIES: &[(&str, &[(&str, &str)])] = &[\n");
    for library in libraries {
        let id = library.file_name().ok_or("missing library folder name")?.to_string_lossy();
        code.push_str(&format!("({id:?}, &[\n"));
        let manifest_path = library.join("library.json");
        if !manifest_path.is_file() {
            return Err(format!("Bundled library `{id}` has no library.json").into());
        }
        let mut files = vec![("library.json".to_owned(), manifest_path)];
        for directory in ["symbols", "footprints", "categories", "parts"] {
            let folder = library.join(directory);
            if !folder.is_dir() {
                continue;
            }
            for entry in fs::read_dir(folder)? {
                let path = entry?.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    let name = path.file_name().ok_or("missing item filename")?.to_string_lossy();
                    files.push((format!("{directory}/{name}"), path));
                }
            }
        }
        files.sort();
        for (name, path) in files {
            println!("cargo:rerun-if-changed={}", path.display());
            code.push_str(&format!("({name:?}, include_str!({:?})),\n", path.display().to_string()));
        }
        code.push_str("]),\n");
    }
    code.push_str("];\n");
    fs::write(Path::new(&env::var("OUT_DIR")?).join("bundled_libraries.rs"), code)?;
    Ok(())
}
