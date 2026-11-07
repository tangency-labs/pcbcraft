//! Embed the starter library folder so the desktop and browser builds open with the same parts.
//! Every `*.json` file in `libraries/starter/symbols` is included, sorted by file name; the
//! engine parses and checks them at startup, so adding a symbol needs no code change.

use std::{env, error::Error, fs, path::Path};

/// Write `starter_library.rs` into `OUT_DIR`, embedding the starter manifest and the JSON symbol
/// files sorted by path. Environment and filesystem errors fail the build; the engine parses
/// and validates the embedded contents at startup.
fn main() -> Result<(), Box<dyn Error>> {
    // Cargo may reuse this executable after the source moves between worktrees.
    // Resolve the invocation's manifest directory rather than the compiler's old path.
    let manifest = env::var_os("CARGO_MANIFEST_DIR").ok_or("Cargo did not provide CARGO_MANIFEST_DIR")?;
    let library = Path::new(&manifest).join("../../libraries/starter");
    let symbols = library.join("symbols");
    println!("cargo:rerun-if-changed={}", library.display());
    println!("cargo:rerun-if-changed={}", symbols.display());
    let mut files = Vec::new();
    for entry in fs::read_dir(&symbols)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "json") {
            files.push(path);
        }
    }
    files.sort();
    let mut code = format!("pub(crate) const MANIFEST: &str = include_str!({:?});\n", library.join("library.json").display().to_string());
    code.push_str("pub(crate) const SYMBOLS: &[(&str, &str)] = &[\n");
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
        code.push_str(&format!("    ({name:?}, include_str!({:?})),\n", path.display().to_string()));
    }
    code.push_str("];\n");
    fs::write(Path::new(&env::var("OUT_DIR")?).join("starter_library.rs"), code)?;
    Ok(())
}
