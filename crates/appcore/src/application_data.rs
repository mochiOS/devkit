//! Private application data. The OS selects the verified application, publisher
//! and account incarnation. Applications supply only relative file names and
//! need no home-directory capability for these operations.
use std::io;

/// A writable directory issued for this app and OS account. It grants no
/// authority over another app, another account, or an older account incarnation.
pub struct ApplicationData {
    #[cfg(target_os = "mochios")]
    native: native::Directory,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !matches!(name, "." | "..")
        && !name
            .bytes()
            .any(|b| b == b'/' || b == b'\\' || b.is_ascii_control())
}
impl ApplicationData {
    /// Ask the OS for this application's private directory. The request has
    /// no application-ID, pathname, username or UID argument.
    pub fn open() -> io::Result<Self> {
        #[cfg(target_os = "mochios")]
        {
            Ok(Self {
                native: native::Directory::open()?,
            })
        }
        #[cfg(not(target_os = "mochios"))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Native application storage is unavailable on this platform",
            ))
        }
    }
    /// Read at most `maximum` bytes from one regular file. Only a genuinely
    /// missing file returns None; permission, I/O and format errors propagate.
    pub fn read(&self, name: &str, maximum: usize) -> io::Result<Option<Vec<u8>>> {
        if !valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid application data name",
            ));
        }
        #[cfg(target_os = "mochios")]
        {
            self.native.read(name, maximum)
        }
        #[cfg(not(target_os = "mochios"))]
        {
            let _ = maximum;
            Err(io::ErrorKind::Unsupported.into())
        }
    }
    /// Replace one file using a synced exclusive temporary file, rename and
    /// directory sync. A failed write never truncates the previous file.
    pub fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        if !valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid application data name",
            ));
        }
        #[cfg(target_os = "mochios")]
        {
            self.native.write(name, bytes)
        }
        #[cfg(not(target_os = "mochios"))]
        {
            let _ = bytes;
            Err(io::ErrorKind::Unsupported.into())
        }
    }
    /// Remove one entry from this application's directory.
    pub fn remove(&self, name: &str) -> io::Result<()> {
        if !valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid application data name",
            ));
        }
        #[cfg(target_os = "mochios")]
        {
            self.native.remove(name)
        }
        #[cfg(not(target_os = "mochios"))]
        {
            Err(io::ErrorKind::Unsupported.into())
        }
    }
}

#[cfg(target_os = "mochios")]
mod native {
    use super::*;
    use mochi_user_platform as p;
    use mochios_posix_client::{
        Client, ClientError, ObjectHandleAttachment, OpenAtBase, StatAtBase,
        SyscallEndpointTransport,
    };
    use mochios_posix_protocol as wire;
    type NativeClient = Client<SyscallEndpointTransport>;
    struct Owned(u32);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = p::handle::close(self.0);
        }
    }
    pub(super) struct Directory {
        root: ObjectHandleAttachment<u32>,
        _owned: Owned,
        client: NativeClient,
    }
    fn native_error(error: p::syscall::SysError) -> io::Error {
        io::Error::from_raw_os_error(mochios_posix_client::newlib_errno::from_status(error.raw()))
    }
    fn remote(error: ClientError<p::syscall::SysError>) -> io::Error {
        match error {
            ClientError::Transport(e) => native_error(e),
            ClientError::Remote(s) => io::Error::from_raw_os_error(
                mochios_posix_client::newlib_errno::from_status(i64::from(s)),
            ),
            _ => io::Error::other("Invalid filesystem response"),
        }
    }
    impl Directory {
        pub fn open() -> io::Result<Self> {
            let broker = u32::try_from(
                p::process::find_by_name("capability.service").map_err(native_error)?,
            )
            .map_err(|_| io::Error::other("Invalid application storage endpoint"))?;
            let mut reply = [0; 8];
            let mut objects = p::ipc::IpcObjectHandles::default();
            let result = (|| {
                let length = p::ipc::call_object_handles(
                    broker,
                    &p::capability::application_data::REQUEST,
                    &mut reply,
                    &mut objects,
                )
                .map_err(native_error)? as u32 as usize;
                if length != 8 || !objects.is_user_delivery() {
                    return Err(io::Error::other("Invalid application storage response"));
                }
                let status = i64::from_le_bytes(reply);
                if status != 0 {
                    return Err(io::Error::from_raw_os_error(
                        mochios_posix_client::newlib_errno::from_status(status),
                    ));
                }
                let object = objects.handles[0];
                let rights = p::handle::HANDLE_RIGHT_READ
                    | p::handle::HANDLE_RIGHT_WRITE
                    | p::handle::HANDLE_RIGHT_DUPLICATE
                    | p::handle::HANDLE_RIGHT_TRANSFER;
                if objects.count != 1
                    || object.handle == 0
                    || object.reserved != 0
                    || object.rights != rights
                {
                    return Err(io::Error::other("Invalid private directory authority"));
                }
                let client = NativeClient::from_syscall_launch_context().map_err(remote)?;
                let root = ObjectHandleAttachment {
                    handle: object.handle,
                    rights,
                };
                let metadata = client.fstat(root).map_err(remote)?;
                let session = client.session_info().map_err(remote)?;
                if metadata.mode & 0o170000 != 0o040000 || metadata.uid != session.effective_uid {
                    return Err(io::Error::other("Private directory owner mismatch"));
                }
                objects.handles[0].handle = 0;
                Ok(Self {
                    root,
                    _owned: Owned(root.handle),
                    client,
                })
            })();
            for object in objects
                .handles
                .iter()
                .take((objects.count as usize).min(objects.handles.len()))
            {
                if object.handle != 0 {
                    let _ = p::handle::close(object.handle);
                }
            }
            result
        }
        pub fn read(&self, name: &str, maximum: usize) -> io::Result<Option<Vec<u8>>> {
            let file = match self.client.open_at(
                OpenAtBase::Directory(self.root),
                wire::OPEN_READ | wire::OPEN_BENEATH | wire::OPEN_NOFOLLOW | wire::OPEN_NONBLOCK,
                0,
                name,
            ) {
                Err(ClientError::Remote(-2)) => return Ok(None),
                result => result.map_err(remote)?,
            };
            let _owned = Owned(file.handle);
            if self.client.fstat(file).map_err(remote)?.mode & 0o170000 != 0o100000 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Application data is not a regular file",
                ));
            }
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            loop {
                let n = p::handle::read(file.handle, &mut buffer).map_err(native_error)?;
                if n == 0 {
                    return Ok(Some(bytes));
                }
                if n > maximum.saturating_sub(bytes.len()) {
                    return Err(io::ErrorKind::FileTooLarge.into());
                }
                bytes.extend_from_slice(&buffer[..n]);
            }
        }
        pub fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
            static NEXT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
            let process = p::process::current_execution_identity()
                .map_err(native_error)?
                .process_id;
            let mut created = None;
            for _ in 0..64 {
                let id = NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                let temporary = format!(".data-write-{process}-{id}");
                match self.client.open_at(
                    OpenAtBase::Directory(self.root),
                    wire::OPEN_WRITE
                        | wire::OPEN_CREATE
                        | wire::OPEN_EXCLUSIVE
                        | wire::OPEN_BENEATH
                        | wire::OPEN_NOFOLLOW,
                    0o600,
                    &temporary,
                ) {
                    Ok(file) => {
                        created = Some((temporary, file));
                        break;
                    }
                    Err(ClientError::Remote(-17)) => continue,
                    Err(error) => return Err(remote(error)),
                }
            }
            let (temporary, file) = created.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "Application data temporary names are busy",
                )
            })?;
            let owned = Owned(file.handle);
            let result = (|| {
                let mut offset = 0;
                while offset < bytes.len() {
                    let n =
                        p::handle::write(file.handle, &bytes[offset..]).map_err(native_error)?;
                    if n == 0 {
                        return Err(io::ErrorKind::WriteZero.into());
                    }
                    offset += n;
                }
                self.client.fsync(file).map_err(remote)?;
                self.client
                    .rename_at(
                        StatAtBase::Directory(self.root),
                        &temporary,
                        StatAtBase::Directory(self.root),
                        name,
                    )
                    .map_err(remote)?;
                self.client.fsync(self.root).map_err(remote)
            })();
            drop(owned);
            if result.is_err() {
                let _ = self
                    .client
                    .unlink_at(StatAtBase::Directory(self.root), 0, &temporary);
            }
            result
        }
        pub fn remove(&self, name: &str) -> io::Result<()> {
            self.client
                .unlink_at(StatAtBase::Directory(self.root), 0, name)
                .map_err(remote)?;
            self.client.fsync(self.root).map_err(remote)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_cannot_select_another_directory_or_reserved_path() {
        for name in [
            "",
            ".",
            "..",
            "/home/user/file",
            "../file",
            "a/b",
            "a\\b",
            "a\0b",
            "a\nb",
        ] {
            assert!(!super::valid_name(name), "{name:?}");
        }
        for name in ["settings.toml", "history-v1", "日本語.txt"] {
            assert!(super::valid_name(name));
        }
    }
}
