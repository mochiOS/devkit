//! Capability-checked access to the shared workspace clipboard.
//!
//! Applications need `clipboard.read` and/or `clipboard.write` in their manifest.

use crate::{Error, Result};

pub const TEXT_CONTENT_TYPE: &str = "text/plain;charset=utf-8";

/// Replaces the clipboard with UTF-8 text.
pub fn set_text(text: &str) -> Result<()> {
    platform::set_text(text)
}

/// Returns UTF-8 text when the clipboard contains a compatible text type.
pub fn text() -> Result<Option<String>> {
    platform::text()
}

/// Replaces the clipboard with bytes and an explicit MIME content type.
pub fn set(content_type: &str, bytes: &[u8]) -> Result<()> {
    if content_type.is_empty() {
        return Err(Error::InvalidArgument);
    }
    platform::set(content_type, bytes)
}

/// Returns the clipboard content type and bytes, or `None` when it is empty.
pub fn content() -> Result<Option<(String, Vec<u8>)>> {
    platform::content()
}

#[cfg(target_os = "mochios")]
mod platform {
    use crate::Result;

    pub fn set_text(text: &str) -> Result<()> {
        mochi_user_platform::workspace::set_clipboard_text(text).map_err(Into::into)
    }

    pub fn text() -> Result<Option<String>> {
        mochi_user_platform::workspace::clipboard_text().map_err(Into::into)
    }

    pub fn set(content_type: &str, bytes: &[u8]) -> Result<()> {
        mochi_user_platform::workspace::set_clipboard(content_type, bytes).map_err(Into::into)
    }

    pub fn content() -> Result<Option<(String, Vec<u8>)>> {
        mochi_user_platform::workspace::clipboard().map_err(Into::into)
    }
}

#[cfg(not(target_os = "mochios"))]
mod platform {
    use crate::{Error, Result};

    pub fn set_text(_: &str) -> Result<()> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn text() -> Result<Option<String>> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn set(_: &str, _: &[u8]) -> Result<()> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn content() -> Result<Option<(String, Vec<u8>)>> {
        Err(Error::UnsupportedPlatform)
    }
}
