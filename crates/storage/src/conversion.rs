//! Explicit, source-preserving conversion of the immediately previous split layout.

use pcbcraft_engine::persistence::{Documents, Encoding, decode_legacy_packed, encode_packed_documents, read_bytes};
use std::{fs, io::Write, path::Path};

/// A source is identified by its validated file type and content, independently of extension.
enum Source<'a> {
    Folder(&'a Path),
    Archive(Vec<u8>),
}

/// Validate an old `project.pc` folder/entry point or container-v2 ZIP, then write a new
/// folder or container-v3 package. Existing destinations and destinations inside the source
/// folder are refused. The source is never recovered, rewritten or adopted as a live project.
/// Folder publication uses the normal recoverable save protocol; interruption can leave a
/// new destination needing recovery, but cannot replace an existing project.
pub fn convert(source: &Path, destination: &Path, encoding: Encoding) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(error)?;
    let input = if metadata.is_dir() {
        Source::Folder(source)
    } else if metadata.is_file() {
        let bytes = read_bytes(fs::File::open(source).map_err(error)?).map_err(error)?;
        if Encoding::detect(&bytes) == Encoding::Zip {
            Source::Archive(bytes)
        } else if source.file_name().is_some_and(|name| name == "project.pc") {
            Source::Folder(source.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new(".")))
        } else {
            return Err("Conversion supports previous split folders and container-v2 ZIPs, not standalone JSON.".to_owned());
        }
    } else {
        return Err("Conversion source must be a regular project file or folder.".to_owned());
    };
    let parent = destination.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(error)?;
    let destination = parent.join(destination.file_name().ok_or("Conversion destination has no filename")?);
    if let Source::Folder(root) = &input {
        let root = fs::canonicalize(root).map_err(error)?;
        if destination.starts_with(&root) {
            return Err("Choose a conversion destination outside the source project folder.".to_owned());
        }
    }
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err("Conversion requires an unused destination; existing files and folders are never replaced.".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(other) => return Err(error(other)),
    }
    let documents = match input {
        Source::Folder(root) => super::folder::load_legacy(root)?,
        Source::Archive(bytes) => decode_legacy_packed(&bytes)?.documents,
    };
    publish(&destination, documents, encoding)
}

/// Publish only validated documents at an unused destination. Archive installation is atomic
/// and refuses a racing destination; a newly created folder uses journaled publication.
pub(super) fn publish(destination: &Path, documents: Documents, encoding: Encoding) -> Result<(), String> {
    if encoding == Encoding::Zip {
        let bytes = encode_packed_documents(documents)?;
        let parent = destination.parent().ok_or("Conversion destination has no parent")?;
        let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
        staged.write_all(&bytes).and_then(|()| staged.as_file().sync_all()).map_err(error)?;
        staged.persist_noclobber(destination).map_err(error)?;
        super::folder::sync_dir(parent)
    } else {
        super::folder::preflight(&documents)?;
        fs::create_dir(destination).map_err(error)?;
        super::folder::sync_dir(destination.parent().ok_or("Conversion destination has no parent")?)?;
        super::folder::save(destination, None, &documents, &mut |_| Ok(()))
    }
}

/// Retain filesystem context through the ordinary CLI error channel.
fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests;
