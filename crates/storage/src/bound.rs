//! Handle-bound document publication. Paths select a directory once; later I/O uses that
//! verified handle, so replacing a board parent with a symlink cannot redirect a write.

#[cfg(unix)]
use rustix::fs::{AtFlags, Mode, OFlags};
#[cfg(not(unix))]
use std::fs;
#[cfg(not(unix))]
use std::path::PathBuf;

use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    sync::Arc,
};

#[derive(Clone)]
pub(super) struct Directory {
    file: Arc<File>,
    #[cfg(not(unix))]
    path: PathBuf,
    // Windows forbids renaming/deleting any captured ancestor while these handles live.
    #[cfg(windows)]
    ancestors: Vec<Arc<File>>,
}

impl Directory {
    /// Capture a non-symlink project root. Its caller supplies the trusted host-selected prefix.
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        let file = File::from(rustix::fs::open(path, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?);
        #[cfg(windows)]
        let file = windows_directory(path)?;
        #[cfg(not(any(unix, windows)))]
        let file = File::open(path)?;
        Ok(Self {
            file: Arc::new(file),
            #[cfg(not(unix))]
            path: path.to_owned(),
            #[cfg(windows)]
            ancestors: Vec::new(),
        })
    }

    /// Open one directory component without following links. Creation never adopts a raced-in
    /// symlink: an AlreadyExists result still goes through the no-follow open and type check.
    pub(super) fn child(&self, name: &str, create: bool) -> io::Result<Option<Self>> {
        component(name)?;
        let open = || {
            #[cfg(unix)]
            return rustix::fs::openat(
                self.file.as_ref(),
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map(File::from)
            .map_err(io::Error::from);
            #[cfg(windows)]
            return windows_directory(&self.path.join(name));
            #[cfg(not(any(unix, windows)))]
            return Err(io::Error::new(io::ErrorKind::Unsupported, "Handle-bound directory traversal is unsupported on this platform"));
        };
        let file = match open() {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                #[cfg(unix)]
                let created = rustix::fs::mkdirat(self.file.as_ref(), name, Mode::from_raw_mode(0o777)).map_err(io::Error::from);
                #[cfg(not(unix))]
                let created = fs::create_dir(self.path.join(name));
                match created {
                    Ok(()) => self.sync()?,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                open()?
            }
            Err(error) => return Err(error),
        };
        Ok(Some(Self {
            file: Arc::new(file),
            #[cfg(not(unix))]
            path: self.path.join(name),
            #[cfg(windows)]
            ancestors: self.ancestors.iter().cloned().chain(std::iter::once(self.file.clone())).collect(),
        }))
    }

    /// Open a regular leaf through this parent and reject symlinks, reparse points and special files.
    /// Nonblocking Unix open prevents a raced-in FIFO from stalling the file worker.
    pub(super) fn open_file(&self, name: &str) -> io::Result<Option<File>> {
        self.open_leaf(name, false)
    }

    /// Confirm the exact bytes and flush the same handle. Windows FlushFileBuffers requires
    /// write access; ordinary reads remain read-only and this open never creates or truncates.
    pub(super) fn confirm_file(&self, name: &str, expected: &[u8]) -> io::Result<()> {
        let file = self.open_leaf(name, cfg!(windows))?.ok_or_else(|| io::Error::other("Published file disappeared"))?;
        let mut bytes = Vec::new();
        (&file).take(expected.len() as u64 + 1).read_to_end(&mut bytes)?;
        if bytes != expected {
            return Err(io::Error::other("Published revision bytes differ"));
        }
        file.sync_all()
    }

    /// Apply identical no-follow and regular-file checks to reading and durable confirmation.
    fn open_leaf(&self, name: &str, writable: bool) -> io::Result<Option<File>> {
        component(name)?;
        #[cfg(unix)]
        let opened = rustix::fs::openat(
            self.file.as_ref(),
            name,
            (if writable { OFlags::RDWR } else { OFlags::RDONLY }) | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(io::Error::from);
        #[cfg(not(unix))]
        let opened = {
            let mut options = fs::OpenOptions::new();
            options.read(true);
            options.write(writable);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.custom_flags(0x00200000);
            }
            options.open(self.path.join(name))
        };
        let file = match opened {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(io::Error::other("Reparse-point document refused"));
            }
        }
        if !metadata.is_file() {
            return Err(io::Error::other("Document is not a regular file"));
        }
        Ok(Some(file))
    }

    /// Read a bounded regular leaf through its captured parent; an absent leaf remains absent.
    pub(super) fn read(&self, name: &str, limit: usize) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = self.open_file(name)? else { return Ok(None) };
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(io::Error::other("Document exceeds size limit"));
        }
        Ok(Some(bytes))
    }

    /// Create an unused recovery temporary without following or truncating an existing leaf.
    pub(super) fn create(&self, name: &str) -> io::Result<File> {
        component(name)?;
        #[cfg(unix)]
        return rustix::fs::openat(
            self.file.as_ref(),
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map(File::from)
        .map_err(io::Error::from);
        #[cfg(not(unix))]
        return fs::OpenOptions::new().write(true).create_new(true).open(self.path.join(name));
    }

    /// Atomically rename a leaf between captured parents. A final symlink is replaced, never followed.
    pub(super) fn rename(&self, name: &str, destination: &Self, new_name: &str) -> io::Result<()> {
        component(name)?;
        component(new_name)?;
        #[cfg(unix)]
        return rustix::fs::renameat(self.file.as_ref(), name, destination.file.as_ref(), new_name).map_err(io::Error::from);
        #[cfg(not(unix))]
        return fs::rename(self.path.join(name), destination.path.join(new_name));
    }

    /// Publish a complete staged file under an unused name. Hard-link creation is atomic and
    /// refuses collisions on all supported hosts, without replacing an immutable revision.
    pub(super) fn publish_new(&self, staged: &str, destination: &Self, name: &str) -> io::Result<()> {
        component(staged)?;
        component(name)?;
        #[cfg(unix)]
        rustix::fs::linkat(self.file.as_ref(), staged, destination.file.as_ref(), name, AtFlags::empty())?;
        #[cfg(not(unix))]
        fs::hard_link(self.path.join(staged), destination.path.join(name))?;
        Ok(())
    }

    /// Open a reusable writable lock through its captured parent, refusing links and devices.
    pub(super) fn lock_file(&self, name: &str) -> io::Result<File> {
        component(name)?;
        #[cfg(unix)]
        let file = File::from(rustix::fs::openat(
            self.file.as_ref(),
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?);
        #[cfg(not(unix))]
        let file = {
            let mut options = fs::OpenOptions::new();
            options.read(true).write(true).create(true).truncate(false);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.custom_flags(0x00200000);
            }
            options.open(self.path.join(name))?
        };
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("Lock is not a regular file"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if file.metadata()?.file_attributes() & 0x400 != 0 {
                return Err(io::Error::other("Reparse-point lock refused"));
            }
        }
        Ok(file)
    }

    /// Remove a leaf entry rather than following it. Missing entries are already removed.
    pub(super) fn remove_file(&self, name: &str) -> io::Result<()> {
        component(name)?;
        #[cfg(unix)]
        let removed = rustix::fs::unlinkat(self.file.as_ref(), name, AtFlags::empty()).map_err(io::Error::from);
        #[cfg(not(unix))]
        let removed = fs::remove_file(self.path.join(name));
        match removed {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    /// Remove only an empty directory entry; no symlink is followed and user contents prevent removal.
    pub(super) fn remove_dir(&self, name: &str) -> io::Result<()> {
        component(name)?;
        #[cfg(unix)]
        return rustix::fs::unlinkat(self.file.as_ref(), name, AtFlags::REMOVEDIR).map_err(io::Error::from);
        #[cfg(not(unix))]
        return fs::remove_dir(self.path.join(name));
    }

    /// Flush the captured directory where supported, without resolving its pathname again.
    pub(super) fn sync(&self) -> io::Result<()> {
        #[cfg(unix)]
        self.file.sync_all()?;
        Ok(())
    }
}

/// Only one portable leaf is accepted by handle-relative operations, never traversal or a prefix.
fn component(name: &str) -> io::Result<()> {
    if name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\\', '\0']) {
        Err(io::Error::other("Invalid directory-relative component"))
    } else {
        Ok(())
    }
}

/// Windows holds every verified directory without FILE_SHARE_DELETE, making subsequent relative
/// path operations stable. OPEN_REPARSE_POINT opens the entry itself, which is then type-checked.
#[cfg(windows)]
fn windows_directory(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let file = fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2).custom_flags(0x02000000 | 0x00200000).open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err(io::Error::other("Not a regular directory"));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;

    /// Leaf symlinks cannot be read or truncated, even after the parent handle is captured.
    #[cfg(unix)]
    #[test]
    fn leaf_links_cannot_redirect_reads_or_creation() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.json");
        std::fs::write(&outside, b"private").unwrap();
        let parent = dir.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        let directory = Directory::open(&parent).unwrap();
        std::os::unix::fs::symlink(&outside, parent.join("board.json")).unwrap();
        assert!(directory.read("board.json", 100).is_err());
        assert!(directory.create("board.json").is_err());
        let mut staged = directory.create("temporary").unwrap();
        staged.write_all(b"replacement").unwrap();
        drop(staged);
        directory.rename("temporary", &directory, "board.json").unwrap();
        assert_eq!(std::fs::read(&outside).unwrap(), b"private");
        assert_eq!(directory.read("board.json", 100).unwrap().unwrap(), b"replacement");
    }

    /// Windows denies ancestor renaming while a verified descendant binding is alive.
    #[cfg(windows)]
    #[test]
    fn captured_ancestors_cannot_be_replaced_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let root = Directory::open(dir.path()).unwrap();
        let boards = root.child("boards", true).unwrap().unwrap();
        let board = boards.child("board-test", true).unwrap().unwrap();
        drop(boards);
        assert!(std::fs::rename(dir.path().join("boards"), dir.path().join("renamed")).is_err());
        drop(board);
        std::fs::rename(dir.path().join("boards"), dir.path().join("renamed")).unwrap();
    }
}
