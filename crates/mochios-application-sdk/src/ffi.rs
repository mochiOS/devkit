//! Stable C ABI used by Clang and future Kome bindings.
//!
//! All strings are borrowed UTF-8 byte spans. Returned variable-length data is
//! copied into caller-owned storage: call once with a null/zero-capacity buffer
//! to obtain the required length, allocate, then call again. No Rust allocation
//! ever crosses the ABI boundary.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::{ptr, slice, str};

use crate::document::AssociationRoles;
use crate::{Error, clipboard, document};

pub const ABI_VERSION_MAJOR: u32 = 1;
pub const ABI_VERSION_MINOR: u32 = 0;
pub const ABI_VERSION_PATCH: u32 = 0;
pub const ABI_VERSION: u32 =
    (ABI_VERSION_MAJOR << 16) | (ABI_VERSION_MINOR << 8) | ABI_VERSION_PATCH;

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    NullPointer = 1,
    InvalidUtf8 = 2,
    InvalidArgument = 3,
    BufferTooSmall = 4,
    UnsupportedPlatform = 5,
    SystemError = 6,
    Panic = 255,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StringView {
    pub data: *const u8,
    pub length: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MutableBuffer {
    pub data: *mut u8,
    pub capacity: u64,
}

thread_local! {
    static LAST_SYSTEM_ERROR: Cell<i64> = const { Cell::new(0) };
}

fn remember_system_error(value: i64) {
    LAST_SYSTEM_ERROR.with(|slot| slot.set(value));
}

fn map_error(error: Error) -> Status {
    match error {
        Error::InvalidArgument => Status::InvalidArgument,
        Error::InvalidUtf8 => Status::InvalidUtf8,
        Error::UnsupportedPlatform => Status::UnsupportedPlatform,
        Error::System(code) => {
            remember_system_error(code);
            Status::SystemError
        }
    }
}

fn ffi_status(operation: impl FnOnce() -> Result<(), Status>) -> i32 {
    remember_system_error(0);
    catch_unwind(AssertUnwindSafe(operation))
        .map(|result| result.map_or_else(|status| status as i32, |_| Status::Ok as i32))
        .unwrap_or(Status::Panic as i32)
}

unsafe fn string<'a>(value: StringView) -> Result<&'a str, Status> {
    let length = usize::try_from(value.length).map_err(|_| Status::InvalidArgument)?;
    if length == 0 {
        return Ok("");
    }
    if value.data.is_null() {
        return Err(Status::NullPointer);
    }
    let bytes = unsafe { slice::from_raw_parts(value.data, length) };
    str::from_utf8(bytes).map_err(|_| Status::InvalidUtf8)
}

unsafe fn write_bytes(
    value: &[u8],
    output: MutableBuffer,
    required_length: *mut u64,
) -> Result<(), Status> {
    if required_length.is_null() {
        return Err(Status::NullPointer);
    }
    let required = u64::try_from(value.len()).map_err(|_| Status::InvalidArgument)?;
    unsafe { *required_length = required };
    if output.capacity < required {
        return Err(Status::BufferTooSmall);
    }
    if value.is_empty() {
        return Ok(());
    }
    if output.data.is_null() {
        return Err(Status::NullPointer);
    }
    unsafe { ptr::copy_nonoverlapping(value.as_ptr(), output.data, value.len()) };
    Ok(())
}

fn roles(bits: u16) -> Result<AssociationRoles, Status> {
    AssociationRoles::from_bits(bits).ok_or(Status::InvalidArgument)
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdk_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdk_last_system_error() -> i64 {
    LAST_SYSTEM_ERROR.with(Cell::get)
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdk_status_name(status: i32) -> StringView {
    let name = match status {
        0 => "ok",
        1 => "null_pointer",
        2 => "invalid_utf8",
        3 => "invalid_argument",
        4 => "buffer_too_small",
        5 => "unsupported_platform",
        6 => "system_error",
        255 => "panic",
        _ => "unknown_status",
    };
    StringView {
        data: name.as_ptr(),
        length: name.len() as u64,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdk_clipboard_set_text(text: StringView) -> i32 {
    ffi_status(|| {
        let text = unsafe { string(text)? };
        clipboard::set_text(text).map_err(map_error)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdk_clipboard_copy_text(
    output: MutableBuffer,
    required_length: *mut u64,
    has_text: *mut u8,
) -> i32 {
    ffi_status(|| {
        if has_text.is_null() {
            return Err(Status::NullPointer);
        }
        unsafe { *has_text = 0 };
        let Some(text) = clipboard::text().map_err(map_error)? else {
            return unsafe { write_bytes(&[], output, required_length) };
        };
        unsafe { *has_text = 1 };
        unsafe { write_bytes(text.as_bytes(), output, required_length) }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdk_association_set(
    extension: StringView,
    content_type: StringView,
    bundle_id: StringView,
    role_bits: u16,
) -> i32 {
    ffi_status(|| {
        document::set_default(
            unsafe { string(extension)? },
            unsafe { string(content_type)? },
            unsafe { string(bundle_id)? },
            roles(role_bits)?,
        )
        .map_err(map_error)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdk_association_remove(
    extension: StringView,
    content_type: StringView,
    role_bits: u16,
) -> i32 {
    ffi_status(|| {
        document::remove_default(
            unsafe { string(extension)? },
            unsafe { string(content_type)? },
            roles(role_bits)?,
        )
        .map_err(map_error)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdk_association_resolve(
    extension: StringView,
    content_type: StringView,
    role_bits: u16,
    output: MutableBuffer,
    required_length: *mut u64,
) -> i32 {
    ffi_status(|| {
        let bundle = document::resolve_default(
            unsafe { string(extension)? },
            unsafe { string(content_type)? },
            roles(role_bits)?,
        )
        .map_err(map_error)?;
        unsafe { write_bytes(bundle.as_bytes(), output, required_length) }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdk_document_open(
    path: StringView,
    content_type: StringView,
    role_bits: u16,
    process_id: *mut u64,
) -> i32 {
    ffi_status(|| {
        if process_id.is_null() {
            return Err(Status::NullPointer);
        }
        let id = document::open(
            unsafe { string(path)? },
            unsafe { string(content_type)? },
            roles(role_bits)?,
        )
        .map_err(map_error)?;
        unsafe { *process_id = id };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdk_document_open_with(
    path: StringView,
    content_type: StringView,
    bundle_id: StringView,
    role_bits: u16,
    process_id: *mut u64,
) -> i32 {
    ffi_status(|| {
        if process_id.is_null() {
            return Err(Status::NullPointer);
        }
        let id = document::open_with(
            unsafe { string(path)? },
            unsafe { string(content_type)? },
            unsafe { string(bundle_id)? },
            roles(role_bits)?,
        )
        .map_err(map_error)?;
        unsafe { *process_id = id };
        Ok(())
    })
}

#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub extern "C" fn mosdk_application_request_exit() -> i32 {
    ffi_status(|| {
        crate::request_exit();
        Ok(())
    })
}

#[cfg(not(feature = "ui"))]
#[unsafe(no_mangle)]
pub extern "C" fn mosdk_application_request_exit() -> i32 {
    Status::UnsupportedPlatform as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_has_stable_encoding() {
        assert_eq!(ABI_VERSION, 0x0001_0000);
        assert_eq!(mosdk_abi_version(), ABI_VERSION);
    }

    #[test]
    fn buffer_query_reports_required_length_without_writing() {
        let mut required = 0;
        let status = unsafe {
            write_bytes(
                b"mochiOS",
                MutableBuffer {
                    data: ptr::null_mut(),
                    capacity: 0,
                },
                &mut required,
            )
        };
        assert_eq!(status, Err(Status::BufferTooSmall));
        assert_eq!(required, 7);
    }

    #[test]
    fn buffer_copy_uses_caller_owned_memory() {
        let mut bytes = [0u8; 7];
        let mut required = 0;
        unsafe {
            write_bytes(
                b"mochiOS",
                MutableBuffer {
                    data: bytes.as_mut_ptr(),
                    capacity: bytes.len() as u64,
                },
                &mut required,
            )
            .unwrap();
        }
        assert_eq!(&bytes, b"mochiOS");
        assert_eq!(required, 7);
    }

    #[test]
    fn ffi_rejects_invalid_utf8_and_role_bits() {
        let invalid = [0xff];
        assert_eq!(
            unsafe {
                mosdk_clipboard_set_text(StringView {
                    data: invalid.as_ptr(),
                    length: 1,
                })
            },
            Status::InvalidUtf8 as i32
        );
        assert_eq!(
            unsafe {
                mosdk_association_remove(
                    StringView {
                        data: b"txt".as_ptr(),
                        length: 3,
                    },
                    StringView {
                        data: ptr::null(),
                        length: 0,
                    },
                    0,
                )
            },
            Status::InvalidArgument as i32
        );
    }
}
