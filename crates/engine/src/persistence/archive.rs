//! Bounded ZIP packaging for the native JSON document.

use std::io::{Cursor, Read, Write};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::MAX_PROJECT_BYTES;

const FORMAT: &str = "pcbcraft-container/1";
const MANIFEST: &str = "manifest.json";
const PROJECT: &str = "project.json";
const MANIFEST_LIMIT: usize = 256 * 1024;
const CHUNK_BYTES: usize = 256 * 1024;
const ZIP_END: &[u8; 4] = b"PK\x05\x06";
const END_SIZE: usize = 22;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    project: Payload,
    assets: Vec<Payload>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    size_bytes: usize,
    sha256: String,
}

/// A bounded ZIP writer. It hashes and copies the JSON in chunks so the browser can
/// yield between steps. The current schema can package project JSON without assets.
pub struct ArchiveEncoder {
    project: Vec<u8>,
    offset: usize,
    hash: Option<Sha256>,
    writer: Option<ZipWriter<Cursor<Vec<u8>>>>,
}

impl ArchiveEncoder {
    /// Start packaging an already encoded native JSON document.
    pub fn new(project: Vec<u8>) -> Result<Self, String> {
        if project.len() > MAX_PROJECT_BYTES {
            return Err("project JSON exceeds the native file size limit".to_owned());
        }
        Ok(Self { project, offset: 0, hash: Some(Sha256::new()), writer: None })
    }

    /// Hash or copy one bounded chunk. Return the archive once its directory is finished.
    pub fn step(&mut self) -> Result<Option<Vec<u8>>, String> {
        let end = (self.offset + CHUNK_BYTES).min(self.project.len());
        if let Some(hash) = &mut self.hash {
            hash.update(&self.project[self.offset..end]);
            self.offset = end;
            if end == self.project.len() {
                let hash = self.hash.take().ok_or("project digest is unavailable")?;
                self.writer = Some(start_archive(self.project.len(), format!("{:x}", hash.finalize()))?);
                self.offset = 0;
            }
            return Ok(None);
        }
        let writer = self.writer.as_mut().ok_or("archive encoding is already complete")?;
        writer.write_all(&self.project[self.offset..end]).map_err(archive_error)?;
        self.offset = end;
        if end < self.project.len() {
            return Ok(None);
        }
        let writer = self.writer.take().ok_or("archive writer is unavailable")?;
        writer.finish().map(|cursor| Some(cursor.into_inner())).map_err(archive_error)
    }
}

/// Start the two ordinary Store entries in a fixed order with reproducible metadata.
fn start_archive(size_bytes: usize, sha256: String) -> Result<ZipWriter<Cursor<Vec<u8>>>, String> {
    let manifest = Manifest { format: FORMAT.to_owned(), project: Payload { size_bytes, sha256 }, assets: Vec::new() };
    let manifest = serde_json::to_vec(&manifest).map_err(archive_error)?;
    let mut writer = ZipWriter::new(Cursor::new(Vec::with_capacity(size_bytes + 512)));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    writer.start_file(MANIFEST, options).map_err(archive_error)?;
    writer.write_all(&manifest).map_err(archive_error)?;
    writer.start_file(PROJECT, options).map_err(archive_error)?;
    Ok(writer)
}

/// Validate the container before handing its exact document bytes to the JSON decoder.
pub(super) fn unpack(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let directory = check_directory(bytes)?;
    let mut zip = ZipArchive::with_config(zip::read::Config { archive_offset: zip::read::ArchiveOffset::Known(0) }, Cursor::new(bytes))
        .map_err(archive_error)?;
    if zip.central_directory_start() != directory || zip.has_duplicate_names() || zip.has_overlapping_entries().map_err(archive_error)? {
        return Err("invalid, duplicate, or overlapping ZIP entries".to_owned());
    }
    if zip.len() != 2 || zip.file_names_raw().any(|name| name != MANIFEST.as_bytes() && name != PROJECT.as_bytes()) {
        return Err("expected only manifest.json and project.json in this project ZIP".to_owned());
    }
    let manifest: Manifest = serde_json::from_slice(&read_entry(&mut zip, MANIFEST, MANIFEST_LIMIT)?).map_err(archive_error)?;
    if manifest.format != FORMAT {
        return Err(format!("unsupported ZIP container format {}", manifest.format));
    }
    if !manifest.assets.is_empty() {
        return Err("bundled assets are unavailable in the current project schema".to_owned());
    }
    if manifest.project.size_bytes > MAX_PROJECT_BYTES {
        return Err("manifest project size exceeds the native file size limit".to_owned());
    }
    let project = read_entry(&mut zip, PROJECT, manifest.project.size_bytes)?;
    let actual = format!("{:x}", Sha256::digest(&project));
    if project.len() != manifest.project.size_bytes || actual != manifest.project.sha256 {
        return Err("project ZIP manifest size or digest does not match project.json".to_owned());
    }
    Ok(project)
}

/// Bound the directory before the ZIP library allocates its metadata. The current
/// format has two entries and needs neither ZIP64 nor split archives.
fn check_directory(bytes: &[u8]) -> Result<u64, String> {
    let start = bytes.len().saturating_sub(END_SIZE + usize::from(u16::MAX));
    let candidates = bytes[start..].windows(END_SIZE).enumerate();
    let mut directory = None;
    for (index, end) in candidates.rev() {
        if &end[..4] != ZIP_END {
            continue;
        }
        let position = start + index;
        let field = |i| u16::from_le_bytes([end[i], end[i + 1]]);
        if position + END_SIZE + usize::from(field(20)) != bytes.len() {
            continue;
        }
        let size = u32::from_le_bytes([end[12], end[13], end[14], end[15]]) as usize;
        let offset = u32::from_le_bytes([end[16], end[17], end[18], end[19]]) as usize;
        if field(4) != 0 || field(6) != 0 || field(8) != 2 || field(10) != 2 || offset.checked_add(size) != Some(position) || directory.is_some() {
            return Err("invalid ZIP directory for a two-entry project archive".to_owned());
        }
        directory = Some(offset as u64);
    }
    directory.ok_or_else(|| "missing ZIP end of directory".to_owned())
}

/// Read a regular, unencrypted file through a limit that detects understated sizes.
fn read_entry(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str, limit: usize) -> Result<Vec<u8>, String> {
    let entry = zip.by_name(name).map_err(archive_error)?;
    let file_type = entry.unix_mode().unwrap_or_default() & 0o170000;
    if !entry.is_file()
        || !matches!(file_type, 0 | 0o100000)
        || entry.encrypted()
        || !matches!(entry.compression(), CompressionMethod::Stored | CompressionMethod::Deflated)
    {
        return Err(format!("unsupported ZIP entry {name}"));
    }
    let declared = entry.size();
    if declared > limit as u64 {
        return Err(format!("ZIP entry {name} exceeds its size limit"));
    }
    let mut bytes = Vec::new();
    entry.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(archive_error)?;
    if bytes.len() > limit || bytes.len() as u64 != declared {
        return Err(format!("ZIP entry {name} has an invalid expanded size"));
    }
    Ok(bytes)
}

fn archive_error(error: impl std::fmt::Display) -> String {
    format!("invalid project ZIP: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid ZIP envelope around arbitrary names and payloads to isolate format validation.
    fn package(method: CompressionMethod, entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer.start_file(*name, SimpleFileOptions::default().compression_method(method)).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// Describe the claimed project size and digest independently of what is stored in the ZIP.
    fn manifest(size_bytes: usize, sha256: String) -> Vec<u8> {
        serde_json::to_vec(&Manifest { format: FORMAT.to_owned(), project: Payload { size_bytes, sha256 }, assets: Vec::new() }).unwrap()
    }

    /// The original project bytes survive Deflate, and checks run after ZIP CRC validation.
    #[test]
    fn checks_manifest_size_and_digest_of_deflated_payload() {
        let project = include_bytes!("../../../app/tests/fixtures/two-resistors.pc");
        let digest = format!("{:x}", Sha256::digest(project));
        let run = |manifest: &[u8], bytes: &[u8]| unpack(&package(CompressionMethod::Deflated, &[(MANIFEST, manifest), (PROJECT, bytes)]));
        let valid = manifest(project.len(), digest.clone());
        assert_eq!(run(&valid, project).unwrap(), project);
        assert!(run(&manifest(project.len(), "0".repeat(64)), project).unwrap_err().contains("digest"));
        assert!(run(&manifest(project.len() - 1, digest.clone()), project).unwrap_err().contains("size"));
        assert!(run(&manifest(project.len() + 1, digest.clone()), project).unwrap_err().contains("size"));
        assert!(run(&manifest(MAX_PROJECT_BYTES + 1, digest), project).unwrap_err().contains("size"));
        let expanded = vec![0u8; MAX_PROJECT_BYTES + 1];
        assert!(run(&valid, &expanded).unwrap_err().contains("size"));
    }

    /// Reject names that cannot be part of the project and duplicate ZIP records.
    #[test]
    fn rejects_foreign_unsafe_and_duplicate_entries() {
        assert!(unpack(&package(CompressionMethod::Stored, &[("notes.txt", b"unrelated ZIP")])).is_err());
        for name in ["../evil.json", "/project.json", "assets/mystery"] {
            let bytes = package(CompressionMethod::Stored, &[(MANIFEST, b"{}"), (name, b"project")]);
            assert!(unpack(&bytes).unwrap_err().contains("expected only"));
        }
        let mut duplicate = package(CompressionMethod::Stored, &[(PROJECT, b"first"), ("another.json", b"second")]);
        for index in (0..duplicate.len() - PROJECT.len()).rev() {
            if &duplicate[index..index + PROJECT.len()] == b"another.json" {
                duplicate[index..index + PROJECT.len()].copy_from_slice(PROJECT.as_bytes());
            }
        }
        assert!(unpack(&duplicate).unwrap_err().contains("duplicate"));
    }

    /// A ZIP symlink with an otherwise permitted name is still unsafe.
    #[test]
    fn rejects_link_entry_at_a_supported_name() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.add_symlink(MANIFEST, "/tmp/manifest.json", SimpleFileOptions::default()).unwrap();
        writer.start_file(PROJECT, SimpleFileOptions::default()).unwrap();
        let archive = writer.finish().unwrap().into_inner();
        assert!(unpack(&archive).unwrap_err().contains("unsupported ZIP entry"));
    }
}
