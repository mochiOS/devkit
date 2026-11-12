//! Spawn explicitly declared, signed helpers without opening installation paths.
//!
//! `executable_export` and per-binary `executable_dependency` declarations are
//! required. Selection and execution remain capability checked by mochiOS.
//! Hosts return `Unsupported`; they never substitute a host executable.
/// POSIX exit status returned after the registered child has been reaped.
pub use std::process::ExitStatus;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::File,
    io,
    path::Path,
    sync::Arc,
};
/// Owned reference to captured signed code and its declared library closure.
/// Clones share ownership; no physical package path is exposed. Package
/// replacement may make later execution fail, but cannot substitute new bytes.
#[derive(Clone, Debug)]
pub struct Executable {
    reference: Arc<native::Reference>,
    name: String,
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && !name
            .bytes()
            .any(|b| b == b'/' || b == b'\\' || b.is_ascii_control())
}
impl Executable {
    /// Select an own-package export explicitly enabled for this binary.
    /// Rejects paths. Requires native signed startup authority and process.spawn.
    pub fn main(name: &str) -> io::Result<Self> {
        Self::select("", name, name)
    }
    /// Select an external executable_dependency by its signed local alias.
    /// Provider identity, key, provenance, ABI and code are verified by the
    /// broker. Installation alone never grants access; no path fallback exists.
    pub fn dependency(alias: &str) -> io::Result<Self> {
        Self::select(alias, "", alias)
    }
    fn select(alias: &str, name: &str, label: &str) -> io::Result<Self> {
        if !valid_name(label) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid executable alias",
            ));
        }
        Ok(Self {
            reference: Arc::new(native::open(alias, name)?),
            name: label.into(),
        })
    }
}
/// Standard stream disposition for a signed helper. Unspecified streams inherit
/// the parent's streams. Piped streams are returned as ordinary owned Files.
#[derive(Debug, Default)]
pub enum Stdio {
    /// Inherit the parent's corresponding standard descriptor.
    #[default]
    Inherit,
    /// Open the native null device under normal OS policy.
    Null,
    /// Create a CLOEXEC pipe and expose the parent's end through Child.
    Piped,
    /// Use a duplicate of this owned file; the caller's file is never closed.
    File(File),
}
impl From<File> for Stdio {
    fn from(file: File) -> Self {
        Self::File(file)
    }
}
/// Builder for an authenticated helper. It inherits the current environment
/// unless env_clear is selected. No environment value can supply authority.
/// Spawning confirms a committed exec; a loader or authorization error is an
/// error from spawn, rather than a fabricated successful child.
#[derive(Debug)]
pub struct Command {
    executable: Executable,
    arguments: Vec<OsString>,
    environment: BTreeMap<OsString, Option<OsString>>,
    clear_environment: bool,
    directory: Option<OsString>,
    streams: [Stdio; 3],
    #[cfg(unix)]
    mappings: Vec<(std::os::fd::OwnedFd, i32)>,
}
impl Command {
    /// Build a command for an owned signed executable selection.
    pub fn new(executable: Executable) -> Self {
        Self {
            executable,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            clear_environment: false,
            directory: None,
            streams: Default::default(),
            #[cfg(unix)]
            mappings: Vec::new(),
        }
    }
    /// Append one literal argument; no shell parsing or expansion is performed.
    pub fn arg(&mut self, value: impl AsRef<OsStr>) -> &mut Self {
        self.arguments.push(value.as_ref().into());
        self
    }
    /// Append literal arguments in their supplied order.
    pub fn args<I, S>(&mut self, values: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for value in values {
            self.arg(value);
        }
        self
    }
    /// Override a child environment variable. Invalid names/NUL fail at spawn.
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.environment
            .insert(key.as_ref().into(), Some(value.as_ref().into()));
        self
    }
    /// Remove one variable from the inherited child environment.
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.environment.insert(key.as_ref().into(), None);
        self
    }
    /// Start with an empty child environment, retaining subsequent overrides.
    pub fn env_clear(&mut self) -> &mut Self {
        self.clear_environment = true;
        self.environment.clear();
        self
    }
    /// Set the working directory through normal POSIX spawn file actions.
    /// This does not add filesystem access to the child or parent.
    pub fn current_dir(&mut self, path: impl AsRef<Path>) -> &mut Self {
        self.directory = Some(path.as_ref().as_os_str().into());
        self
    }
    /// Configure the child's standard input.
    pub fn stdin(&mut self, stream: impl Into<Stdio>) -> &mut Self {
        self.streams[0] = stream.into();
        self
    }
    /// Configure the child's standard output.
    pub fn stdout(&mut self, stream: impl Into<Stdio>) -> &mut Self {
        self.streams[1] = stream.into();
        self
    }
    /// Configure the child's standard error.
    pub fn stderr(&mut self, stream: impl Into<Stdio>) -> &mut Self {
        self.streams[2] = stream.into();
        self
    }
    /// Pin a duplicate of an explicitly supplied descriptor and map it in the
    /// child. Targets 0..2 use stdin/stdout/stderr; extra targets must be >=3.
    /// Duplicate targets fail. Sources are moved above all targets before spawn
    /// so overlapping descriptor numbers cannot change another mapping.
    #[cfg(unix)]
    pub fn inherit_fd(
        &mut self,
        source: std::os::fd::BorrowedFd<'_>,
        target: i32,
    ) -> io::Result<&mut Self> {
        if !(3..4096).contains(&target) || self.mappings.iter().any(|(_, old)| *old == target) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid or duplicate descriptor target",
            ));
        }
        self.mappings.push((source.try_clone_to_owned()?, target));
        Ok(self)
    }
    /// Spawn a registered child after authenticating code and attenuating its
    /// permissions. Requires process.spawn, the normal exec runtime's system.random.read,
    /// and a matching updated OS/runtime. No pre_exec callback
    /// is executed in application code. Errors preserve parent descriptors.
    pub fn spawn(&mut self) -> io::Result<Child> {
        native::spawn(self)
    }
    fn child_environment(&self) -> BTreeMap<OsString, OsString> {
        let mut values = if self.clear_environment {
            BTreeMap::new()
        } else {
            std::env::vars_os().collect()
        };
        for (key, value) in &self.environment {
            if let Some(value) = value {
                values.insert(key.clone(), value.clone());
            } else {
                values.remove(key);
            }
        }
        values
    }
}
/// Owned child process and optional parent pipe ends. Call wait/try_wait to
/// reap the child. Dropping Child closes references but neither terminates nor
/// waits, matching ordinary process ownership conventions.
#[derive(Debug)]
pub struct Child {
    process: native::Process,
    /// Writable parent end when the child uses piped stdin.
    pub stdin: Option<File>,
    /// Readable parent end when the child uses piped stdout.
    pub stdout: Option<File>,
    /// Readable parent end when the child uses piped stderr.
    pub stderr: Option<File>,
}
impl Child {
    /// Return the POSIX child identifier for display; it is not an authority.
    pub fn id(&self) -> u32 {
        self.process.id()
    }
    /// Wait and reap exactly this child. Repeated calls return its cached status.
    /// Close/take piped stdin first if the child is waiting for input EOF.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.process
            .wait(false)?
            .ok_or_else(|| io::Error::other("missing child exit status"))
    }
    /// Reap an exited child without blocking, or return None while running.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.process.wait(true)
    }
    /// Force termination through the owned native ProcessHandle, never a PID
    /// lookup. This is native termination with exit code 137, not delivery of a
    /// catchable POSIX signal. Still call wait to reap the child.
    pub fn terminate(&mut self) -> io::Result<()> {
        self.process.terminate()
    }
}
#[cfg(not(target_os = "mochios"))]
mod native {
    use super::*;
    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "signed executable authority requires native mochiOS",
        )
    }
    #[derive(Debug)]
    pub struct Reference;
    #[derive(Debug)]
    pub struct Process;
    pub fn open(_: &str, _: &str) -> io::Result<Reference> {
        Err(unsupported())
    }
    pub fn spawn(_: &Command) -> io::Result<Child> {
        Err(unsupported())
    }
    impl Process {
        pub fn id(&self) -> u32 {
            0
        }
        pub fn wait(&mut self, _: bool) -> io::Result<Option<ExitStatus>> {
            Err(unsupported())
        }
        pub fn terminate(&mut self) -> io::Result<()> {
            Err(unsupported())
        }
    }
}
#[cfg(target_os = "mochios")]
#[path = "process/native.rs"]
mod native;
#[cfg(all(test, not(target_os = "mochios")))]
mod tests {
    use super::*;
    fn command() -> Command {
        Command::new(Executable {
            reference: Arc::new(native::Reference),
            name: "fixture".into(),
        })
    }
    #[test]
    fn invalid_names_are_not_paths_or_empty_selectors() {
        for name in ["", "..", "/bin/sh", "a/b", "a\\b", "x\n"] {
            assert!(!valid_name(name));
        }
    }
    #[test]
    fn hosts_do_not_substitute_untrusted_executables() {
        assert_eq!(
            Executable::main("worker").unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            Executable::dependency("worker").unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }
    #[test]
    fn environment_clear_remove_and_literal_arguments() {
        let mut c = command();
        c.arg("a b")
            .arg("$(x)")
            .env_clear()
            .env("ONE", "1")
            .env("TWO", "2")
            .env_remove("ONE");
        let e = c.child_environment();
        assert_eq!(e.len(), 1);
        assert_eq!(e.get(OsStr::new("TWO")).unwrap(), "2");
        assert_eq!(c.arguments, [OsString::from("a b"), OsString::from("$(x)")]);
    }
    #[cfg(unix)]
    #[test]
    fn descriptor_sources_are_owned_and_targets_unique() {
        use std::os::fd::AsFd;
        let file = File::open("/dev/null").unwrap();
        let mut c = command();
        assert!(c.inherit_fd(file.as_fd(), 2).is_err());
        c.inherit_fd(file.as_fd(), 7).unwrap();
        assert!(c.inherit_fd(file.as_fd(), 7).is_err());
        drop(file);
        assert!(c.mappings[0].0.as_fd().try_clone_to_owned().is_ok());
        assert_eq!(c.spawn().unwrap_err().kind(), io::ErrorKind::Unsupported);
    }
}
