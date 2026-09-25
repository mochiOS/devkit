use core::fmt;

pub type Result<T> = core::result::Result<T, Error>;

/// An application-facing error that does not expose service implementation details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidArgument,
    InvalidUtf8,
    UnsupportedPlatform,
    System(i64),
}

impl Error {
    /// Returns the positive OS error number when the failure came from mochiOS.
    pub const fn system_code(self) -> Option<i64> {
        match self {
            Self::System(code) => Some(code),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument => formatter.write_str("invalid argument"),
            Self::InvalidUtf8 => formatter.write_str("invalid UTF-8"),
            Self::UnsupportedPlatform => formatter.write_str("unsupported platform"),
            Self::System(code) => write!(formatter, "mochiOS error {code}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(target_os = "mochios")]
impl From<mochi_user_platform::syscall::SysError> for Error {
    fn from(error: mochi_user_platform::syscall::SysError) -> Self {
        Self::System(error.raw().unsigned_abs() as i64)
    }
}
