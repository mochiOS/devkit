use super::*;
use mochi_user_platform as platform;
use std::{
    ffi::{CString, c_char, c_int, c_void},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, process::ExitStatusExt},
    },
    ptr,
};
unsafe extern "C" {
    fn mochi_executable_open(alias: *const c_char, name: *const c_char) -> u32;
    fn mochi_executable_close(reference: u32) -> c_int;
    fn mochi_executable_spawn(
        output: *mut Spawned,
        reference: u32,
        actions: *const *mut c_void,
        attr: *const c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    fn posix_spawn_file_actions_init(actions: *mut *mut c_void) -> c_int;
    fn posix_spawn_file_actions_destroy(actions: *mut *mut c_void) -> c_int;
    fn posix_spawn_file_actions_adddup2(
        actions: *mut *mut c_void,
        source: c_int,
        target: c_int,
    ) -> c_int;
    fn posix_spawn_file_actions_addclose(actions: *mut *mut c_void, fd: c_int) -> c_int;
    fn posix_spawn_file_actions_addchdir_np(
        actions: *mut *mut c_void,
        path: *const c_char,
    ) -> c_int;
}
#[repr(C)]
struct Spawned {
    pid: i32,
    process: u32,
}
#[derive(Debug)]
pub(super) struct Reference(u32);
impl Drop for Reference {
    fn drop(&mut self) {
        unsafe {
            mochi_executable_close(self.0);
        }
    }
}
fn cstring(value: &OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in process argument or environment",
        )
    })
}
pub(super) fn open(alias: &str, name: &str) -> io::Result<Reference> {
    let a = CString::new(alias).map_err(|_| io::ErrorKind::InvalidInput)?;
    let n = CString::new(name).map_err(|_| io::ErrorKind::InvalidInput)?;
    let handle = unsafe { mochi_executable_open(a.as_ptr(), n.as_ptr()) };
    if handle == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(Reference(handle))
    }
}
fn status(result: c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}
struct Actions(*mut c_void);
impl Drop for Actions {
    fn drop(&mut self) {
        unsafe {
            posix_spawn_file_actions_destroy(&mut self.0);
        }
    }
}
impl Actions {
    fn new() -> io::Result<Self> {
        let mut actions = ptr::null_mut();
        status(unsafe { posix_spawn_file_actions_init(&mut actions) })?;
        Ok(Self(actions))
    }
    fn dup(&mut self, source: i32, target: i32) -> io::Result<()> {
        status(unsafe { posix_spawn_file_actions_adddup2(&mut self.0, source, target) })
    }
    fn close(&mut self, fd: i32) -> io::Result<()> {
        status(unsafe { posix_spawn_file_actions_addclose(&mut self.0, fd) })
    }
}
fn pin(source: i32, floor: i32) -> io::Result<File> {
    let fd = unsafe { libc::fcntl(source, libc::F_DUPFD_CLOEXEC, floor) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn pipe() -> io::Result<(File, File)> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) })
}
fn native_error(error: platform::syscall::SysError) -> io::Error {
    io::Error::from_raw_os_error(mochios_posix_client::newlib_errno::from_status(error.raw()))
}
#[derive(Debug)]
pub(super) struct Process {
    pid: i32,
    handle: u32,
    status: Option<ExitStatus>,
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = platform::handle::close(self.handle);
    }
}
impl Process {
    pub(super) fn id(&self) -> u32 {
        self.pid as u32
    }
    pub(super) fn wait(&mut self, nonblocking: bool) -> io::Result<Option<ExitStatus>> {
        if self.status.is_some() {
            return Ok(self.status);
        }
        loop {
            let mut raw = 0;
            let result = unsafe {
                libc::waitpid(
                    self.pid,
                    &mut raw,
                    if nonblocking { libc::WNOHANG } else { 0 },
                )
            };
            if result == self.pid {
                self.status = Some(ExitStatus::from_raw(raw));
                return Ok(self.status);
            }
            if result == 0 && nonblocking {
                return Ok(None);
            }
            let error = io::Error::last_os_error();
            if result == -1 && error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(if result == -1 {
                error
            } else {
                io::Error::other("invalid registered child wait result")
            });
        }
    }
    pub(super) fn terminate(&mut self) -> io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        let status = platform::process::handle_status(self.handle).map_err(native_error)?;
        if status.state == platform::syscall::PROCESS_STATE_EXITED {
            return Ok(());
        }
        platform::process::terminate_handle(self.handle, 137).map_err(native_error)
    }
}
pub(super) fn spawn(command: &Command) -> io::Result<Child> {
    let mut argv = vec![
        CString::new(command.executable.name.as_bytes())
            .map_err(|_| io::ErrorKind::InvalidInput)?,
    ];
    for argument in &command.arguments {
        argv.push(cstring(argument)?);
    }
    let mut env = Vec::new();
    for (key, value) in command.child_environment() {
        if key.is_empty() || key.as_bytes().contains(&b'=') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid environment name",
            ));
        }
        let mut entry = key;
        entry.push("=");
        entry.push(value);
        env.push(cstring(&entry)?);
    }
    let mut args: Vec<_> = argv.iter().map(|v| v.as_ptr()).collect();
    args.push(ptr::null());
    let mut vars: Vec<_> = env.iter().map(|v| v.as_ptr()).collect();
    vars.push(ptr::null());
    let directory = command.directory.as_deref().map(cstring).transpose()?;
    let floor = command
        .mappings
        .iter()
        .map(|(_, target)| *target)
        .max()
        .unwrap_or(2)
        + 1;
    let mut actions = Actions::new()?;
    if let Some(directory) = &directory {
        status(unsafe {
            posix_spawn_file_actions_addchdir_np(&mut actions.0, directory.as_ptr())
        })?;
    }
    let mut retained = Vec::new();
    let mut parent: [Option<File>; 3] = Default::default();
    for (index, stream) in command.streams.iter().enumerate() {
        let file = match stream {
            Stdio::Inherit => continue,
            Stdio::File(file) => pin(file.as_raw_fd(), floor)?,
            Stdio::Null => {
                let source = std::fs::OpenOptions::new()
                    .read(index == 0)
                    .write(index != 0)
                    .open("/dev/null")?;
                pin(source.as_raw_fd(), floor)?
            }
            Stdio::Piped => {
                let (read, write) = pipe()?;
                let (child, end) = if index == 0 {
                    (read, write)
                } else {
                    (write, read)
                };
                let pinned = pin(child.as_raw_fd(), floor)?;
                parent[index] = Some(end);
                pinned
            }
        };
        actions.dup(file.as_raw_fd(), index as i32)?;
        retained.push(file);
    }
    for (source, target) in &command.mappings {
        let file = pin(source.as_raw_fd(), floor)?;
        actions.dup(file.as_raw_fd(), *target)?;
        retained.push(file);
    }
    for file in &retained {
        actions.close(file.as_raw_fd())?;
    }
    let mut result = Spawned { pid: 0, process: 0 };
    status(unsafe {
        mochi_executable_spawn(
            &mut result,
            command.executable.reference.0,
            &actions.0,
            ptr::null(),
            args.as_ptr(),
            vars.as_ptr(),
        )
    })?;
    if result.pid <= 0 || result.process == 0 {
        if result.process != 0 {
            let _ = platform::handle::close(result.process);
        }
        return Err(io::Error::other("invalid spawned child reference"));
    }
    let process = Process {
        pid: result.pid,
        handle: result.process,
        status: None,
    };
    let [stdin, stdout, stderr] = parent;
    Ok(Child {
        process,
        stdin,
        stdout,
        stderr,
    })
}
