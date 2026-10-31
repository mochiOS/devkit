//! Same-filesystem output staging shared by developer tools.
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A staging directory, created atomically with mode 0700 on Unix.
/// Drop removes its remaining files; it never removes a published destination.
pub struct PrivateDirectory(PathBuf);

impl PrivateDirectory {
    pub fn new_in(parent: &Path, prefix: &str) -> io::Result<Self> {
        if prefix.is_empty()
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid temporary directory prefix",
            ));
        }
        for _ in 0..128 {
            let path = parent.join(format!(
                ".{prefix}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cannot create a unique staging directory",
        ))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    fn file_path(&self, name: &str) -> io::Result<PathBuf> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['/', '\\'])
            || !matches!(
                Path::new(name).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staging file must have one name",
            ));
        }
        Ok(self.0.join(name))
    }

    pub fn create_file(&self, name: &str) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        options.open(self.file_path(name)?)
    }

    /// Sync, atomically rename, then sync the destination directory on Unix.
    /// A final directory-sync error occurs after the new file is visible.
    pub fn publish(&self, name: &str, file: &File, destination: &Path) -> io::Result<()> {
        let source = self.file_path(name)?;
        let named = fs::symlink_metadata(&source)?;
        let opened = file.metadata()?;
        if !named.is_file() || !opened.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "output is not a regular file",
            ));
        }
        #[cfg(unix)]
        if named.dev() != opened.dev() || named.ino() != opened.ino() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "output descriptor does not match its name",
            ));
        }
        // Keep using the owned descriptor even if final permissions remove read
        // access. Reopening the pathname would add a new permission requirement.
        file.sync_all()?;
        fs::rename(source, destination)?;
        #[cfg(unix)]
        {
            let parent = destination
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
