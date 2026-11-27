//! Explicit, source-preserving conversion of the immediately previous split layout, and
//! publication of a project made elsewhere, such as an import, at a new destination.

use pcbcraft_engine::Project;
use pcbcraft_engine::persistence::{Documents, Encoding, decode_legacy_packed, encode_documents, encode_packed_documents, read_bytes};
use std::sync::Arc;
use std::{fs, io::Write, path::Path};

/// A source is identified by its validated file type and content, independently of extension.
enum Source<'a> {
    Folder(&'a Path),
    Archive(Vec<u8>),
}

/// Validate an old `project.pc` folder/entry point or container-v2 ZIP, then write a new
/// folder or container-v3 package. Existing destinations and destinations inside the source
/// folder are refused, as are destinations Windows reads as a device on that platform. The
/// source is never recovered, rewritten or adopted as a live project.
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
    let destination = canonical_destination(destination)?;
    if let Source::Folder(root) = &input {
        let root = fs::canonicalize(root).map_err(error)?;
        if destination.starts_with(&root) {
            return Err("Choose a conversion destination outside the source project folder.".to_owned());
        }
    }
    refuse_existing(&destination, "Conversion requires an unused destination; existing files and folders are never replaced.")?;
    let documents = match input {
        Source::Folder(root) => super::folder::load_legacy(root)?,
        Source::Archive(bytes) => decode_legacy_packed(&bytes)?.documents,
    };
    publish(&destination, documents, encoding)
}

/// Write `project` as a new project at an unused `destination`: a ZIP package for
/// [`Encoding::Zip`], else a new folder. Existing files and folders are never replaced, and
/// destinations Windows reads as a device are refused on that platform.
/// The project's layout is prepared as the editor's first Save prepares it, so opening the
/// result and saving it unchanged rewrites nothing.
pub fn create(project: Arc<Project>, destination: &Path, encoding: Encoding) -> Result<(), String> {
    let destination = canonical_destination(destination)?;
    refuse_existing(&destination, "Choose an unused destination; existing files and folders are never replaced.")?;
    let project = super::layout::prepare(project, (encoding == Encoding::Json).then_some(destination.as_path()), None)?;
    publish(&destination, encode_documents(project)?, encoding)
}

/// The encoding a destination asks for by its name: a ZIP package for a `.pc` name in any
/// case, else a project folder.
pub fn encoding_for(destination: &Path) -> Encoding {
    if destination.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pc")) { Encoding::Zip } else { Encoding::Json }
}

/// `destination` under its canonical parent folder, refusing names Windows reads as a device.
fn canonical_destination(destination: &Path) -> Result<std::path::PathBuf, String> {
    let parent = destination.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(error)?;
    let destination = parent.join(destination.file_name().ok_or("The destination has no filename")?);
    refuse_device_destination(&destination)?;
    Ok(destination)
}

/// Refuse a destination that exists in any form, with `message`; other failures to look are errors too.
fn refuse_existing(destination: &Path, message: &str) -> Result<(), String> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(message.to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(other) => Err(error(other)),
    }
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

/// On Windows, refuse a destination whose name before its first dot is a reserved device, such
/// as `con.pc`, `aux` or `COM¹.pc`, ignoring ASCII case and trailing ASCII spaces in the stem.
/// The canonical `\\?\` parent would let the destination be created, leaving a file or folder
/// that ordinary Windows tools cannot open or delete, while
/// opening it by its usual path reaches the device (#156). Other platforms accept these names.
fn refuse_device_destination(destination: &Path) -> Result<(), String> {
    if cfg!(windows)
        && let Some(device) = destination.file_name().and_then(|name| name.to_str()).and_then(super::folder::windows_device_name)
    {
        return Err(format!(
            "Windows reserves the name \"{device}\" for a device, so the converted project can't be written there. Choose another destination name."
        ));
    }
    Ok(())
}

/// Retain filesystem context through the ordinary CLI error channel.
fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests;
