//! Generate canonical split-format test fixtures from the checked-in legacy project.
use pcbcraft_engine::persistence::{decode, encode_documents, encode_packed};
use std::{
    fs,
    io::{Cursor, Write},
    path::PathBuf,
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

/// Write fixtures into an explicitly supplied, previously nonexistent directory.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let destination = std::env::args_os().nth(1).map(PathBuf::from).ok_or("usage: storage_fixtures NEW_OUTPUT_DIRECTORY")?;
    fs::create_dir(&destination)?;
    let project = decode(include_bytes!("../../app/tests/fixtures/two-resistors.pc"))?;
    let documents = encode_documents(project.clone())?;
    for (path, bytes) in &documents {
        let path = destination.join("two-resistors").join(path);
        fs::create_dir_all(path.parent().ok_or("document has no parent")?)?;
        fs::write(path, bytes)?;
    }
    fs::write(destination.join("two-resistors.pc.zip"), encode_packed(project)?)?;
    let mut hand_zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).last_modified_time(zip::DateTime::default());
    hand_zip.add_directory("two-resistors/", options)?;
    for (path, bytes) in documents {
        hand_zip.start_file(format!("two-resistors/{path}"), options)?;
        hand_zip.write_all(&bytes)?;
    }
    fs::write(destination.join("two-resistors-no-manifest.pc.zip"), hand_zip.finish()?.into_inner())?;
    Ok(())
}
