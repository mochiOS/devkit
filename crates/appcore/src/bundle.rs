//! Read-only assets declared by the application's signed package manifest.
//!
//! A `library_dependency` authorizes code only. External data additionally need
//! a named `resource_export`, a pinned `resource_dependency`, and selection in
//! the launching executable's `binary.resource_dependencies` list. An alias is
//! not a package name or an installation path. These APIs never request broad
//! filesystem permissions. Hosts without native startup authority return
//! [`std::io::ErrorKind::Unsupported`], without a filesystem fallback.
use std::{
    ffi::CString,
    fs::File,
    io,
    path::{Path, PathBuf},
};

/// Access to one resource set selected for the currently running executable.
/// Cloning this value copies its alias; it grants no additional permissions.
#[derive(Clone, Debug)]
pub struct Bundle {
    alias: CString,
}
impl Bundle {
    /// Access the main application's declared assets, including its icon.
    /// Requires native authority installed by the trusted application launcher.
    /// Missing/invalid startup authority fails rather than inspecting argv/envp.
    pub fn main() -> io::Result<Self> {
        Self::checked("")
    }
    /// Access an explicitly selected external export by its signed local alias.
    /// The provider's package, publisher, key, provenance and data contract are
    /// checked at launch. Installed but undeclared packages remain inaccessible.
    pub fn dependency(alias: &str) -> io::Result<Self> {
        if alias.is_empty() || alias.len() > 128 || alias.contains('/') || !valid_name(alias) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid resource dependency alias",
            ));
        }
        Self::checked(alias)
    }
    fn checked(alias: &str) -> io::Result<Self> {
        let alias = CString::new(alias)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid resource alias"))?;
        native::check(&alias)?;
        Ok(Self { alias })
    }
    /// Open a declared relative logical name as an ordinary read-only file.
    /// The broker captures and checks its exact signed size and SHA-256 before
    /// returning it. A package update cannot silently substitute newer bytes.
    /// Absolute paths, empty/dot components and traversal are rejected.
    pub fn open(&self, name: &str) -> io::Result<File> {
        if !valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid logical resource name",
            ));
        }
        let name = CString::new(name).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid logical resource name")
        })?;
        native::open(&self.alias, &name)
    }
    /// Capture the entire selected set as an immutable process-local directory.
    /// Use this for libraries needing normal path-based file APIs. The view
    /// contains only explicitly declared files and synthesized directories.
    /// Keep the returned owner alive for as long as any code uses its path.
    /// This can fail at the bounded snapshot size or live-view limits.
    pub fn directory(&self) -> io::Result<ResourceDirectory> {
        native::directory(&self.alias)
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 4095
        && !name.bytes().any(|b| b == b'\\' || b.is_ascii_control())
        && name
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != ".." && p.len() <= 255)
}
/// Owns an immutable process-local directory view. Dropping it invalidates the
/// path; already-open files retain their own contents. The path cannot be
/// serialized, passed to another process or reused after this owner's lifetime.
/// Fork children must create their own views or receive explicit file references.
#[derive(Debug)]
pub struct ResourceDirectory {
    path: PathBuf,
    token: u64,
}
impl ResourceDirectory {
    /// Borrow a path usable by normal open/stat/readdir and read-only mmap.
    /// Canonicalization preserves this view's spelling and authority. No
    /// physical installation directory or neighboring file is made accessible.
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for ResourceDirectory {
    fn drop(&mut self) {
        native::close_directory(self.token);
    }
}
#[cfg(target_os = "mochios")]
mod native {
    use super::*;
    use std::os::fd::FromRawFd;
    unsafe extern "C" {
        fn mochi_bundle_check(alias: *const core::ffi::c_char) -> i32;
        fn mochi_bundle_open(
            alias: *const core::ffi::c_char,
            name: *const core::ffi::c_char,
        ) -> i32;
        fn mochi_bundle_directory_open(
            alias: *const core::ffi::c_char,
            path: *mut core::ffi::c_char,
            capacity: usize,
        ) -> u64;
        fn mochi_bundle_directory_close(token: u64) -> i32;
    }
    pub(super) fn check(alias: &CString) -> io::Result<()> {
        if unsafe { mochi_bundle_check(alias.as_ptr()) } < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub(super) fn open(alias: &CString, name: &CString) -> io::Result<File> {
        let fd = unsafe { mochi_bundle_open(alias.as_ptr(), name.as_ptr()) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    pub(super) fn directory(alias: &CString) -> io::Result<ResourceDirectory> {
        let mut path = [0u8; 128];
        let token = unsafe {
            mochi_bundle_directory_open(alias.as_ptr(), path.as_mut_ptr().cast(), path.len())
        };
        if token == 0 {
            return Err(io::Error::last_os_error());
        }
        let parsed = path
            .iter()
            .position(|b| *b == 0)
            .and_then(|n| core::str::from_utf8(&path[..n]).ok())
            .filter(|p| p.starts_with("/.__mochi_resources/") && !p.contains(".."));
        match parsed {
            Some(path) => Ok(ResourceDirectory {
                path: path.into(),
                token,
            }),
            None => {
                close_directory(token);
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid resource view response",
                ))
            }
        }
    }
    pub(super) fn close_directory(token: u64) {
        let _ = unsafe { mochi_bundle_directory_close(token) };
    }
}
#[cfg(not(target_os = "mochios"))]
mod native {
    use super::*;
    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "native signed bundle authority is unavailable on this host",
        )
    }
    pub(super) fn check(_: &CString) -> io::Result<()> {
        Err(unsupported())
    }
    pub(super) fn open(_: &CString, _: &CString) -> io::Result<File> {
        Err(unsupported())
    }
    pub(super) fn directory(_: &CString) -> io::Result<ResourceDirectory> {
        Err(unsupported())
    }
    pub(super) fn close_directory(_: u64) {}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logical_names_never_select_ambient_files() {
        for name in [
            "",
            "/tmp/file",
            "../file",
            "a/../b",
            "a//b",
            "./file",
            "a\\b",
            "nul\0",
            "a/.",
            "a/..",
        ] {
            assert!(!valid_name(name));
        }
        for name in ["appicon.png", "fonts/regular.ttf", "data/日本語.txt"] {
            assert!(valid_name(name));
        }
    }
    #[cfg(not(target_os = "mochios"))]
    #[test]
    fn preview_does_not_treat_host_paths_as_authenticated_bundles() {
        assert_eq!(
            Bundle::main().unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            Bundle::dependency("/tmp").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            Bundle::dependency("fonts").unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }
}
