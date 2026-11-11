//! Portable archives of the same documents used in an unpacked project folder.

use super::{Documents, MAX_ARCHIVE_BYTES, MAX_DOCUMENT_BYTES, MAX_DOCUMENTS, MAX_DOCUMENTS_BYTES, Project, decode_documents, encode_documents};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Cursor, Read, Write},
    sync::Arc,
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const FORMAT: &str = "pcbcraft-container/3";
const MANIFEST: &str = "manifest.json";
const MANIFEST_LIMIT: usize = 1024 * 1024;
const CHUNK: usize = 256 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    parts: Vec<Part>,
    assets: Vec<Part>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Part {
    path: String,
    size_bytes: usize,
    sha256: String,
}

/// A fully validated current archive. A missing manifest is added on the next save.
pub struct PackedProject {
    /// The project after schema and cross-document validation.
    pub project: Arc<Project>,
    /// Exact split-document bytes for lossless conversion.
    pub documents: Documents,
    /// Whether the archive needs a current manifest on export (missing or explicitly converted).
    pub needs_manifest: bool,
}

/// Encode a portable container v3 with a fixed `project.json` entry point.
pub fn encode_packed(project: Arc<Project>) -> Result<Vec<u8>, String> {
    encode_packed_documents(encode_documents(project)?)
}

/// Package already validated split documents without changing their text formatting.
pub fn encode_packed_documents(documents: Documents) -> Result<Vec<u8>, String> {
    let mut encoder = PackedEncoder::new(documents)?;
    loop {
        if let Some(bytes) = encoder.step()? {
            return Ok(bytes);
        }
    }
}

/// Hash and package documents in bounded chunks so a browser download can yield between frames.
pub struct PackedEncoder {
    documents: Documents,
    paths: Vec<String>,
    parts: Vec<Part>,
    index: usize,
    offset: usize,
    hash: Sha256,
    writer: Option<ZipWriter<Cursor<Vec<u8>>>>,
    finished: bool,
}

impl PackedEncoder {
    /// Retain encoded documents after checking count, path and size limits.
    pub fn new(documents: Documents) -> Result<Self, String> {
        super::documents::check_documents(&documents)?;
        let paths = documents.keys().cloned().collect();
        Ok(Self { documents, paths, parts: Vec::new(), index: 0, offset: 0, hash: Sha256::new(), writer: None, finished: false })
    }

    /// Hash or copy one chunk, emitting a deterministic manifest and fixed ZIP metadata.
    pub fn step(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self.finished {
            return Err("packed encoding is already complete".to_owned());
        }
        if self.index == self.paths.len() {
            if let Some(writer) = self.writer.take() {
                let bytes = writer.finish().map_err(error)?.into_inner();
                if bytes.len() > MAX_ARCHIVE_BYTES {
                    return Err("project ZIP exceeds its size limit".to_owned());
                }
                self.finished = true;
                return Ok(Some(bytes));
            }
            let manifest = serde_json::to_vec(&Manifest { format: FORMAT.to_owned(), parts: std::mem::take(&mut self.parts), assets: Vec::new() })
                .map_err(error)?;
            if manifest.len() > MANIFEST_LIMIT {
                return Err("project manifest exceeds its size limit".to_owned());
            }
            let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
            writer.start_file(MANIFEST, options()).map_err(error)?;
            writer.write_all(&manifest).map_err(error)?;
            self.writer = Some(writer);
            self.index = 0;
            return Ok(None);
        }
        let path = &self.paths[self.index];
        let bytes = &self.documents[path];
        let end = (self.offset + CHUNK).min(bytes.len());
        if let Some(writer) = &mut self.writer {
            if self.offset == 0 {
                writer.start_file(path, options()).map_err(error)?;
            }
            writer.write_all(&bytes[self.offset..end]).map_err(error)?;
        } else {
            self.hash.update(&bytes[self.offset..end]);
            if end == bytes.len() {
                self.parts.push(Part {
                    path: path.clone(),
                    size_bytes: bytes.len(),
                    sha256: format!("{:x}", std::mem::take(&mut self.hash).finalize()),
                });
            }
        }
        self.offset = end;
        if end == bytes.len() {
            self.index += 1;
            self.offset = 0;
        }
        Ok(None)
    }
}

/// Fixed entry metadata makes archives reproducible without affecting working-folder diffs.
fn options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Stored).last_modified_time(zip::DateTime::default()).unix_permissions(0o644)
}

/// Read current or previous split archives. Previous manifests are verified before migration
/// in memory, and their replacement is deferred until Save. Missing manifests
/// are allowed, but malformed or mismatched ones are never ignored. Only manifest-less archives discard ordinary OS
/// metadata, after validating its ZIP paths, types, CRCs and size limits along with every entry.
pub fn decode_packed(bytes: &[u8]) -> Result<PackedProject, String> {
    decode_layout(bytes, FORMAT, super::ENTRY_POINT).or_else(|current_error| decode_legacy_packed(bytes).map_err(|_| current_error))
}

/// Read a previous split container-v2 archive for automatic migration or explicit conversion. Manifest hashes
/// are checked against old paths before renaming the entry point and upgrading power markers;
/// unaffected document bytes survive.
/// Manifest-less previous split ZIPs are also accepted, but standalone containers remain refused.
pub fn decode_legacy_packed(bytes: &[u8]) -> Result<PackedProject, String> {
    decode_layout(bytes, "pcbcraft-container/2", "project.pc")
}

/// Share ZIP bounds, safe-path and integrity validation across explicit layout boundaries.
fn decode_layout(bytes: &[u8], format: &str, entry_point: &str) -> Result<PackedProject, String> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("project ZIP exceeds its size limit".to_owned());
    }
    let directory = check_directory(bytes)?;
    let mut zip =
        ZipArchive::with_config(zip::read::Config { archive_offset: zip::read::ArchiveOffset::Known(0) }, Cursor::new(bytes)).map_err(error)?;
    if zip.central_directory_start() != directory || zip.has_duplicate_names() || zip.has_overlapping_entries().map_err(error)? {
        return Err("invalid, duplicate, or overlapping ZIP entries".to_owned());
    }
    let mut entries = Documents::new();
    let mut directories = Vec::new();
    let mut total = 0;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(error)?;
        check_local_header(bytes, usize::try_from(entry.header_start()).map_err(error)?)?;
        let name = std::str::from_utf8(entry.name_raw()).map_err(error)?.to_owned();
        let path = name.strip_suffix('/').unwrap_or(&name);
        if path.is_empty()
            || path.contains('\\')
            || path.split('/').any(|p| p.is_empty() || p == "." || p == ".." || p.contains(':') || p.chars().any(char::is_control))
        {
            return Err(format!("{name}: unsafe ZIP path"));
        }
        let file_type = entry.unix_mode().unwrap_or_default() & 0o170000;
        if entry.encrypted()
            || !matches!(entry.compression(), CompressionMethod::Stored | CompressionMethod::Deflated)
            || !(if entry.is_dir() { matches!(file_type, 0 | 0o040000) } else { matches!(file_type, 0 | 0o100000) })
        {
            return Err(format!("{name}: unsupported ZIP entry"));
        }
        if entry.is_dir() {
            if entry.size() != 0 {
                return Err(format!("{name}: directory has a payload"));
            }
            let mut payload = Vec::new();
            entry.take(1).read_to_end(&mut payload).map_err(error)?;
            if !payload.is_empty() {
                return Err(format!("{name}: directory has a payload"));
            }
            directories.push(name);
            continue;
        }
        let limit = if path.ends_with(MANIFEST) { MANIFEST_LIMIT } else { MAX_DOCUMENT_BYTES };
        let size = entry.size();
        if size > limit as u64 || total + size > (MAX_DOCUMENTS_BYTES + MANIFEST_LIMIT) as u64 {
            return Err(format!("{name}: ZIP payload exceeds its size limit"));
        }
        let mut payload = Vec::new();
        entry.take(limit as u64 + 1).read_to_end(&mut payload).map_err(|e| format!("{name}: {e}"))?;
        if payload.len() as u64 != size {
            return Err(format!("{name}: ZIP expanded size mismatch"));
        }
        total += size;
        entries.insert(name, payload);
    }
    // Establish the container boundary before root discovery: neither an old entry point nor
    // a missing new entry point may disguise a present unsupported or malformed manifest.
    if let Some(manifest) = entries.get(MANIFEST) {
        let header: serde_json::Value = serde_json::from_slice(manifest).map_err(error)?;
        if header["format"].as_str() != Some(format) {
            return Err(format!("manifest.json: unsupported container format {}; expected {format}", header["format"]));
        }
    }
    // Any manifest prevents metadata filtering, including malformed or misplaced manifests.
    // Filter before root discovery so Apple's sidecar tree cannot masquerade as a second root.
    let has_manifest = entries.keys().any(|name| name.rsplit('/').next() == Some(MANIFEST));
    if !has_manifest {
        entries.retain(|name, _| !is_apple_metadata(name) && !matches!(name.rsplit('/').next(), Some(".DS_Store" | "Thumbs.db" | "desktop.ini")));
        directories.retain(|name| !is_apple_metadata(name));
    }
    let prefix = if entries.contains_key(entry_point) {
        String::new()
    } else {
        let roots: Vec<_> =
            entries.keys().filter_map(|name| name.strip_suffix(&format!("/{entry_point}"))).filter(|root| !root.contains('/')).collect();
        if roots.len() != 1 {
            return Err(format!("{entry_point}: ZIP must contain one project root"));
        }
        format!("{}/", roots[0])
    };
    let mut documents = Documents::new();
    for (name, payload) in entries {
        let path = name.strip_prefix(&prefix).ok_or_else(|| format!("{name}: entry outside the project root"))?;
        documents.insert(path.to_owned(), payload);
    }
    for name in directories {
        if !prefix.is_empty() && name == prefix {
            continue;
        }
        let path = name.strip_prefix(&prefix).ok_or_else(|| format!("{name}: directory outside the project root"))?;
        if !documents.keys().any(|file| file.starts_with(path)) && !matches!(path, "sheets/" | "library/" | "library/symbols/" | "assets/") {
            return Err(format!("{name}: unexpected ZIP directory"));
        }
    }
    let manifest = documents.remove(MANIFEST);
    if let Some(bytes) = &manifest {
        let manifest: Manifest = serde_json::from_slice(bytes).map_err(|e| format!("manifest.json: {e}"))?;
        if manifest.format != format {
            return Err(format!("manifest.json: unsupported container format {}", manifest.format));
        }
        if !manifest.assets.is_empty() {
            return Err("manifest.json: bundled assets are not yet supported".to_owned());
        }
        let mut listed = BTreeSet::new();
        for part in manifest.parts {
            if !listed.insert(part.path.clone()) {
                return Err(format!("manifest.json: duplicate part {}", part.path));
            }
            let bytes = documents.get(&part.path).ok_or_else(|| format!("{}: missing manifest part", part.path))?;
            if bytes.len() != part.size_bytes || format!("{:x}", Sha256::digest(bytes)) != part.sha256 {
                return Err(format!("{}: manifest size or digest mismatch", part.path));
            }
        }
        if let Some(path) = documents.keys().find(|path| !listed.contains(*path)) {
            return Err(format!("{path}: unlisted manifest part"));
        }
    }
    let legacy = entry_point != super::ENTRY_POINT;
    let original_documents = documents.clone();
    let documents = if legacy { super::convert_legacy_documents(documents)? } else { super::migrate_power_markers(documents)? };
    let migrated = documents != original_documents;
    let project = decode_documents(&documents)?;
    Ok(PackedProject { project, documents, needs_manifest: migrated || legacy || manifest.is_none() })
}

/// Recognize Apple's reserved sidecar directory at the archive root or inside its wrapper.
fn is_apple_metadata(path: &str) -> bool {
    path.rsplit_once('/').is_some_and(|(parents, _)| parents.split('/').any(|component| component == "__MACOSX"))
}

/// Bound central-directory allocation, reject multi-disk/ZIP64 and require a complete ordinary
/// ZIP directory before asking the ZIP reader to allocate entries or decompress payloads.
fn check_directory(bytes: &[u8]) -> Result<u64, String> {
    let start = bytes.len().saturating_sub(22 + usize::from(u16::MAX));
    let mut result = None;
    for (index, end) in bytes[start..].windows(22).enumerate().rev() {
        if &end[..4] != b"PK\x05\x06" {
            continue;
        }
        let position = start + index;
        let short = |i| u16::from_le_bytes([end[i], end[i + 1]]);
        if position + 22 + usize::from(short(20)) != bytes.len() {
            continue;
        }
        let long = |i| u32::from_le_bytes([end[i], end[i + 1], end[i + 2], end[i + 3]]) as usize;
        let (count, size, offset) = (short(10) as usize, long(12), long(16));
        if short(4) != 0
            || short(6) != 0
            || short(8) != short(10)
            || count == 0
            || count > MAX_DOCUMENTS * 2
            || offset.checked_add(size) != Some(position)
            || result.is_some()
        {
            return Err("invalid or unsupported ZIP directory".to_owned());
        }
        let mut cursor = offset;
        for _ in 0..count {
            let header = bytes.get(cursor..cursor + 46).ok_or("truncated ZIP directory")?;
            let u16_at = |i| u16::from_le_bytes([header[i], header[i + 1]]) as usize;
            if &header[..4] != b"PK\x01\x02"
                || u16_at(34) != 0
                || header[20..24] == [255; 4]
                || header[24..28] == [255; 4]
                || header[42..46] == [255; 4]
            {
                return Err("ZIP64 or split ZIP is unsupported".to_owned());
            }
            let extra_start = cursor + 46 + u16_at(28);
            let extra_end = extra_start + u16_at(30);
            check_extra(bytes.get(extra_start..extra_end).ok_or("truncated ZIP extra data")?)?;
            cursor = extra_end + u16_at(32);
        }
        if cursor != position {
            return Err("ZIP directory size mismatch".to_owned());
        }
        result = Some(offset as u64);
    }
    result.ok_or_else(|| "missing ZIP end of directory".to_owned())
}

/// Check local headers as well as the central directory: small ZIP64 entries can advertise
/// their extended sizes only in the local extra data, even when the end directory is ordinary.
fn check_local_header(bytes: &[u8], offset: usize) -> Result<(), String> {
    let header = bytes.get(offset..offset.saturating_add(30)).ok_or("truncated ZIP local header")?;
    if &header[..4] != b"PK\x03\x04" || header[18..22] == [255; 4] || header[22..26] == [255; 4] {
        return Err("invalid or ZIP64 local header".to_owned());
    }
    let name_length = u16::from_le_bytes([header[26], header[27]]) as usize;
    let extra_length = u16::from_le_bytes([header[28], header[29]]) as usize;
    let start = offset + 30 + name_length;
    check_extra(bytes.get(start..start + extra_length).ok_or("truncated local ZIP extra data")?)
}

/// Extra fields must be well formed and cannot enable ZIP64 in either header location.
fn check_extra(mut extra: &[u8]) -> Result<(), String> {
    while !extra.is_empty() {
        if extra.len() < 4 {
            return Err("invalid ZIP extra data".to_owned());
        }
        let tag = u16::from_le_bytes([extra[0], extra[1]]);
        let length = u16::from_le_bytes([extra[2], extra[3]]) as usize;
        if tag == 1 {
            return Err("ZIP64 is unsupported".to_owned());
        }
        extra = extra.get(4 + length..).ok_or("invalid ZIP extra data length")?;
    }
    Ok(())
}

/// Preserve the container context on ZIP library and serialization errors.
fn error(e: impl std::fmt::Display) -> String {
    format!("invalid project ZIP: {e}")
}
