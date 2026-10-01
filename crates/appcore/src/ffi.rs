//! Stable C ABI used by Clang and future Kome bindings.
//!
//! All strings are borrowed UTF-8 byte spans. Returned variable-length data is
//! copied into caller-owned storage: call once with a null/zero-capacity buffer
//! to obtain the required length, allocate, then call again. No Rust allocation
//! ever crosses the ABI boundary.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::rc::Rc;
use std::{ptr, slice, str};

use crate::document::AssociationRoles;
use crate::{Error, UserNotification, clipboard, content_type::ContentType, document};

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
    static LAST_STATUS: Cell<i32> = const { Cell::new(Status::Ok as i32) };
    static LAST_RESULT_STRING: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static LAST_RESULT_PRESENT: Cell<u8> = const { Cell::new(0) };
    static LAST_RESULT_U64: Cell<u64> = const { Cell::new(0) };
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

fn map_io_error(error: std::io::Error) -> Status {
    remember_system_error(error.raw_os_error().map_or(-1, i64::from));
    Status::SystemError
}

fn ffi_status(operation: impl FnOnce() -> Result<(), Status>) -> i32 {
    remember_system_error(0);
    let status = catch_unwind(AssertUnwindSafe(operation))
        .map(|result| result.map_or_else(|status| status as i32, |_| Status::Ok as i32))
        .unwrap_or(Status::Panic as i32);
    LAST_STATUS.with(|slot| slot.set(status));
    status
}

fn ffi_pointer<T>(operation: impl FnOnce() -> Result<T, Status>) -> *mut T {
    remember_system_error(0);
    let result = catch_unwind(AssertUnwindSafe(operation));
    match result {
        Ok(Ok(value)) => {
            LAST_STATUS.with(|slot| slot.set(Status::Ok as i32));
            Box::into_raw(Box::new(value))
        }
        Ok(Err(status)) => {
            LAST_STATUS.with(|slot| slot.set(status as i32));
            ptr::null_mut()
        }
        Err(_) => {
            LAST_STATUS.with(|slot| slot.set(Status::Panic as i32));
            ptr::null_mut()
        }
    }
}

fn remember_string(value: Option<&str>) {
    LAST_RESULT_STRING.with(|slot| {
        let mut bytes = slot.borrow_mut();
        bytes.clear();
        if let Some(value) = value {
            bytes.extend_from_slice(value.as_bytes());
        }
    });
    LAST_RESULT_PRESENT.with(|slot| slot.set(value.is_some() as u8));
}

fn remember_u64(value: u64) {
    LAST_RESULT_U64.with(|slot| slot.set(value));
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
pub extern "C" fn mochios_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn mochios_last_system_error() -> i64 {
    LAST_SYSTEM_ERROR.with(Cell::get)
}

/// Returns the status produced by the latest AppCore ABI call on this thread.
#[unsafe(no_mangle)]
pub extern "C" fn mochios_last_status() -> i32 {
    LAST_STATUS.with(Cell::get)
}

/// Returns whether the latest UTF-8 result contains a value.
#[unsafe(no_mangle)]
pub extern "C" fn mochios_last_result_has_value() -> u8 {
    LAST_RESULT_PRESENT.with(Cell::get)
}

/// Returns a borrowed pointer to the latest UTF-8 result.
///
/// The pointer remains valid until another AppCore call stores a string result
/// on the same thread.
#[unsafe(no_mangle)]
pub extern "C" fn mochios_last_result_string_data() -> *const u8 {
    LAST_RESULT_STRING.with(|slot| slot.borrow().as_ptr())
}

/// Returns the byte length of the latest UTF-8 result.
#[unsafe(no_mangle)]
pub extern "C" fn mochios_last_result_string_length() -> usize {
    LAST_RESULT_STRING.with(|slot| slot.borrow().len())
}

/// Returns the latest integer result produced by a Kome adapter call.
#[unsafe(no_mangle)]
pub extern "C" fn mochios_last_result_u64() -> u64 {
    LAST_RESULT_U64.with(Cell::get)
}

#[unsafe(no_mangle)]
pub extern "C" fn mochios_status_name(status: i32) -> StringView {
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
pub unsafe extern "C" fn mochios_clipboard_set_text(text: StringView) -> i32 {
    ffi_status(|| {
        let text = unsafe { string(text)? };
        clipboard::set_text(text).map_err(map_error)
    })
}

/// Replaces the clipboard with UTF-8 bytes passed as a pointer and length.
///
/// This form avoids a by-value C structure and is the stable entry point used
/// by Kome's thin AppCore package.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_clipboard_set_text_utf8(data: *const u8, length: usize) -> i32 {
    let Ok(length) = u64::try_from(length) else {
        return Status::InvalidArgument as i32;
    };
    unsafe { mochios_clipboard_set_text(StringView { data, length }) }
}

/// Reads clipboard text into the thread-local UTF-8 result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn mochios_clipboard_read_text_utf8() -> i32 {
    remember_string(None);
    ffi_status(|| {
        let value = clipboard::text().map_err(map_error)?;
        remember_string(value.as_deref());
        Ok(())
    })
}

/// Validates and canonicalizes a content type into the UTF-8 result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_content_type_parse_utf8(data: *const u8, length: usize) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let value = unsafe {
            string(StringView {
                data,
                length: length as u64,
            })?
        };
        let value = ContentType::parse(value).map_err(map_error)?;
        remember_string(Some(value.identifier()));
        Ok(())
    })
}

/// Resolves a path to its canonical content type in the UTF-8 result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_content_type_for_path_utf8(data: *const u8, length: usize) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let path = unsafe {
            string(StringView {
                data,
                length: length as u64,
            })?
        };
        let value = ContentType::for_path(path);
        remember_string(Some(value.identifier()));
        Ok(())
    })
}

/// Resolves an extension to its canonical content type in the UTF-8 result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_content_type_for_extension_utf8(
    data: *const u8,
    length: usize,
) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let extension = unsafe {
            string(StringView {
                data,
                length: length as u64,
            })?
        };
        let value = ContentType::from_extension(extension);
        remember_string(Some(value.identifier()));
        Ok(())
    })
}

/// Reports whether one validated content type conforms to another.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_content_type_conforms_utf8(
    value_data: *const u8,
    value_length: usize,
    parent_data: *const u8,
    parent_length: usize,
) -> u8 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe {
            string(StringView {
                data: value_data,
                length: value_length as u64,
            })
        }
        .ok()
        .and_then(|value| ContentType::parse(value).ok());
        let parent = unsafe {
            string(StringView {
                data: parent_data,
                length: parent_length as u64,
            })
        }
        .ok()
        .and_then(|value| ContentType::parse(value).ok());
        value
            .zip(parent)
            .is_some_and(|(value, parent)| value.conforms_to(&parent)) as u8
    }));
    result.unwrap_or(0)
}

/// Stores the preferred extension for a validated content type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_content_type_preferred_extension_utf8(
    data: *const u8,
    length: usize,
) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let value = unsafe {
            string(StringView {
                data,
                length: length as u64,
            })?
        };
        let value = ContentType::parse(value).map_err(map_error)?;
        remember_string(value.preferred_extension());
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_clipboard_copy_text(
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
pub unsafe extern "C" fn mochios_association_set(
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
pub unsafe extern "C" fn mochios_association_remove(
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
pub unsafe extern "C" fn mochios_association_resolve(
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

/// Sets a document association from pointer-length UTF-8 arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_association_set_utf8(
    extension_data: *const u8,
    extension_length: usize,
    content_type_data: *const u8,
    content_type_length: usize,
    bundle_id_data: *const u8,
    bundle_id_length: usize,
    role_bits: u16,
) -> i32 {
    ffi_status(|| {
        document::set_default(
            unsafe {
                string(StringView {
                    data: extension_data,
                    length: extension_length as u64,
                })?
            },
            unsafe {
                string(StringView {
                    data: content_type_data,
                    length: content_type_length as u64,
                })?
            },
            unsafe {
                string(StringView {
                    data: bundle_id_data,
                    length: bundle_id_length as u64,
                })?
            },
            roles(role_bits)?,
        )
        .map_err(map_error)
    })
}

/// Removes a document association from pointer-length UTF-8 arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_association_remove_utf8(
    extension_data: *const u8,
    extension_length: usize,
    content_type_data: *const u8,
    content_type_length: usize,
    role_bits: u16,
) -> i32 {
    ffi_status(|| {
        document::remove_default(
            unsafe {
                string(StringView {
                    data: extension_data,
                    length: extension_length as u64,
                })?
            },
            unsafe {
                string(StringView {
                    data: content_type_data,
                    length: content_type_length as u64,
                })?
            },
            roles(role_bits)?,
        )
        .map_err(map_error)
    })
}

/// Resolves a document association into the UTF-8 result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_association_resolve_utf8(
    extension_data: *const u8,
    extension_length: usize,
    content_type_data: *const u8,
    content_type_length: usize,
    role_bits: u16,
) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let value = document::resolve_default(
            unsafe {
                string(StringView {
                    data: extension_data,
                    length: extension_length as u64,
                })?
            },
            unsafe {
                string(StringView {
                    data: content_type_data,
                    length: content_type_length as u64,
                })?
            },
            roles(role_bits)?,
        )
        .map_err(map_error)?;
        remember_string(Some(&value));
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_document_open(
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
pub unsafe extern "C" fn mochios_document_open_with(
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

/// Opens a document from pointer-length UTF-8 arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_document_open_utf8(
    path_data: *const u8,
    path_length: usize,
    content_type_data: *const u8,
    content_type_length: usize,
    role_bits: u16,
) -> i32 {
    remember_u64(0);
    ffi_status(|| {
        let process_id = document::open(
            unsafe {
                string(StringView {
                    data: path_data,
                    length: path_length as u64,
                })?
            },
            unsafe {
                string(StringView {
                    data: content_type_data,
                    length: content_type_length as u64,
                })?
            },
            roles(role_bits)?,
        )
        .map_err(map_error)?;
        remember_u64(process_id);
        Ok(())
    })
}

/// Opens a document with an explicit application from UTF-8 arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_document_open_with_utf8(
    path_data: *const u8,
    path_length: usize,
    content_type_data: *const u8,
    content_type_length: usize,
    bundle_id_data: *const u8,
    bundle_id_length: usize,
    role_bits: u16,
) -> i32 {
    remember_u64(0);
    ffi_status(|| {
        let process_id = document::open_with(
            unsafe {
                string(StringView {
                    data: path_data,
                    length: path_length as u64,
                })?
            },
            unsafe {
                string(StringView {
                    data: content_type_data,
                    length: content_type_length as u64,
                })?
            },
            unsafe {
                string(StringView {
                    data: bundle_id_data,
                    length: bundle_id_length as u64,
                })?
            },
            roles(role_bits)?,
        )
        .map_err(map_error)?;
        remember_u64(process_id);
        Ok(())
    })
}

/// Delivers a user notification from pointer-length UTF-8 arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_notification_deliver_utf8(
    bundle_id_data: *const u8,
    bundle_id_length: usize,
    title_data: *const u8,
    title_length: usize,
    body_data: *const u8,
    body_length: usize,
) -> i32 {
    remember_u64(0);
    ffi_status(|| {
        let notification = UserNotification::new(
            unsafe {
                string(StringView {
                    data: bundle_id_data,
                    length: bundle_id_length as u64,
                })?
            },
            unsafe {
                string(StringView {
                    data: title_data,
                    length: title_length as u64,
                })?
            },
        )
        .body(unsafe {
            string(StringView {
                data: body_data,
                length: body_length as u64,
            })?
        });
        let identifier = notification.deliver().map_err(map_error)?;
        remember_u64(identifier);
        Ok(())
    })
}

/// Mutable builder owned by the Kome Control Center wrapper.
#[cfg(feature = "ui")]
pub struct ControlCenterItemHandle {
    bundle_id: String,
    item_id: String,
    title: String,
    rows: Vec<(String, String)>,
}

/// Creates a Control Center item builder from UTF-8 identifiers.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_control_center_item_create_utf8(
    bundle_id_data: *const u8,
    bundle_id_length: usize,
    item_id_data: *const u8,
    item_id_length: usize,
) -> *mut ControlCenterItemHandle {
    ffi_pointer(|| {
        Ok(ControlCenterItemHandle {
            bundle_id: unsafe {
                string(StringView {
                    data: bundle_id_data,
                    length: bundle_id_length as u64,
                })?
            }
            .to_owned(),
            item_id: unsafe {
                string(StringView {
                    data: item_id_data,
                    length: item_id_length as u64,
                })?
            }
            .to_owned(),
            title: String::new(),
            rows: Vec::new(),
        })
    })
}

/// Sets the title on a Control Center item builder.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_control_center_item_set_title_utf8(
    handle: *mut ControlCenterItemHandle,
    data: *const u8,
    length: usize,
) -> i32 {
    ffi_status(|| {
        let handle = unsafe { handle.as_mut() }.ok_or(Status::NullPointer)?;
        handle.title = unsafe {
            string(StringView {
                data,
                length: length as u64,
            })?
        }
        .to_owned();
        Ok(())
    })
}

/// Appends a row to a Control Center item builder.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_control_center_item_add_row_utf8(
    handle: *mut ControlCenterItemHandle,
    label_data: *const u8,
    label_length: usize,
    value_data: *const u8,
    value_length: usize,
) -> i32 {
    ffi_status(|| {
        let handle = unsafe { handle.as_mut() }.ok_or(Status::NullPointer)?;
        let label = unsafe {
            string(StringView {
                data: label_data,
                length: label_length as u64,
            })?
        }
        .to_owned();
        let value = unsafe {
            string(StringView {
                data: value_data,
                length: value_length as u64,
            })?
        }
        .to_owned();
        handle.rows.push((label, value));
        Ok(())
    })
}

/// Publishes the current Control Center item builder.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_control_center_item_publish(
    handle: *mut ControlCenterItemHandle,
) -> i32 {
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        let mut card = crate::ControlCenterCard::new(&handle.title);
        for (label, value) in &handle.rows {
            card = card.row(label, value);
        }
        crate::ControlCenterItem::register(&handle.bundle_id, &handle.item_id)
            .card(card)
            .publish()
            .map_err(map_error)
    })
}

/// Destroys a Control Center item builder.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_control_center_item_destroy(handle: *mut ControlCenterItemHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Owned AppCore alert used by the Kome wrapper.
#[cfg(feature = "ui")]
pub struct AlertHandle(crate::Alert);

/// Creates an application alert.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub extern "C" fn mochios_alert_create() -> *mut AlertHandle {
    ffi_pointer(|| Ok(AlertHandle(crate::Alert::new())))
}

/// Presents an error using an application alert.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_alert_present_error_utf8(
    handle: *mut AlertHandle,
    title_data: *const u8,
    title_length: usize,
    message_data: *const u8,
    message_length: usize,
) -> i32 {
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        handle.0.present_error(
            unsafe {
                string(StringView {
                    data: title_data,
                    length: title_length as u64,
                })?
            },
            unsafe {
                string(StringView {
                    data: message_data,
                    length: message_length as u64,
                })?
            },
        );
        Ok(())
    })
}

/// Dismisses an application alert.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_alert_dismiss(handle: *mut AlertHandle) -> i32 {
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        handle.0.dismiss();
        Ok(())
    })
}

/// Returns whether an application alert is visible.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_alert_is_visible(handle: *const AlertHandle) -> u8 {
    unsafe { handle.as_ref() }.is_some_and(|handle| handle.0.is_visible()) as u8
}

/// Destroys an application alert.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_alert_destroy(handle: *mut AlertHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Owned open panel used by the Kome wrapper.
#[cfg(feature = "ui")]
pub struct OpenPanelHandle {
    panel: crate::OpenPanel,
    selected: Rc<RefCell<Option<String>>>,
    cancelled: Rc<Cell<bool>>,
}

/// Creates an open panel from pointer-length UTF-8 options.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_open_panel_create_utf8(
    title_data: *const u8,
    title_length: usize,
    initial_data: *const u8,
    initial_length: usize,
    root_data: *const u8,
    root_length: usize,
) -> *mut OpenPanelHandle {
    ffi_pointer(|| {
        let title = unsafe {
            string(StringView {
                data: title_data,
                length: title_length as u64,
            })?
        };
        let initial = unsafe {
            string(StringView {
                data: initial_data,
                length: initial_length as u64,
            })?
        };
        let root = unsafe {
            string(StringView {
                data: root_data,
                length: root_length as u64,
            })?
        };
        let selected = Rc::new(RefCell::new(None));
        let selected_callback = Rc::clone(&selected);
        let cancelled = Rc::new(Cell::new(false));
        let cancelled_callback = Rc::clone(&cancelled);
        let panel = crate::OpenPanel::new_with_cancel(
            crate::OpenPanelOptions {
                title: title.to_owned(),
                initial_directory: (!initial.is_empty()).then(|| PathBuf::from(initial)),
                root_directory: (!root.is_empty()).then(|| PathBuf::from(root)),
                allowed_content_types: Vec::new(),
            },
            move |path| {
                *selected_callback.borrow_mut() = Some(path.to_string_lossy().into_owned());
                Ok(())
            },
            move || cancelled_callback.set(true),
        );
        Ok(OpenPanelHandle {
            panel,
            selected,
            cancelled,
        })
    })
}

/// Presents an open panel.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_open_panel_show(handle: *mut OpenPanelHandle) -> i32 {
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        handle.panel.show();
        Ok(())
    })
}

/// Returns whether an open panel is visible.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_open_panel_is_visible(handle: *const OpenPanelHandle) -> u8 {
    unsafe { handle.as_ref() }.is_some_and(|handle| handle.panel.is_visible()) as u8
}

/// Moves the latest open-panel selection into the UTF-8 result buffer.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_open_panel_take_selection(handle: *mut OpenPanelHandle) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        let selected = handle.selected.borrow_mut().take();
        remember_string(selected.as_deref());
        Ok(())
    })
}

/// Returns and clears the open panel's cancellation flag.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_open_panel_take_cancelled(handle: *mut OpenPanelHandle) -> u8 {
    unsafe { handle.as_ref() }.is_some_and(|handle| handle.cancelled.replace(false)) as u8
}

/// Destroys an open panel.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_open_panel_destroy(handle: *mut OpenPanelHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Owned save panel used by the Kome wrapper.
#[cfg(feature = "ui")]
pub struct SavePanelHandle {
    panel: crate::SavePanel,
    selected: Rc<RefCell<Option<String>>>,
    cancelled: Rc<Cell<bool>>,
}

/// Creates a save panel from pointer-length UTF-8 options.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_save_panel_create_utf8(
    title_data: *const u8,
    title_length: usize,
    suggested_data: *const u8,
    suggested_length: usize,
    initial_data: *const u8,
    initial_length: usize,
    root_data: *const u8,
    root_length: usize,
    confirms_replacement: u8,
) -> *mut SavePanelHandle {
    ffi_pointer(|| {
        let title = unsafe {
            string(StringView {
                data: title_data,
                length: title_length as u64,
            })?
        };
        let suggested_name = unsafe {
            string(StringView {
                data: suggested_data,
                length: suggested_length as u64,
            })?
        };
        let initial = unsafe {
            string(StringView {
                data: initial_data,
                length: initial_length as u64,
            })?
        };
        let root = unsafe {
            string(StringView {
                data: root_data,
                length: root_length as u64,
            })?
        };
        let selected = Rc::new(RefCell::new(None));
        let selected_callback = Rc::clone(&selected);
        let cancelled = Rc::new(Cell::new(false));
        let cancelled_callback = Rc::clone(&cancelled);
        let panel = crate::SavePanel::new_with_cancel(
            crate::SavePanelOptions {
                title: title.to_owned(),
                suggested_name: suggested_name.to_owned(),
                initial_directory: (!initial.is_empty()).then(|| PathBuf::from(initial)),
                root_directory: (!root.is_empty()).then(|| PathBuf::from(root)),
                confirms_replacement: confirms_replacement != 0,
                allowed_content_types: Vec::new(),
            },
            move |path| {
                *selected_callback.borrow_mut() = Some(path.to_string_lossy().into_owned());
                Ok(())
            },
            move || cancelled_callback.set(true),
        );
        Ok(SavePanelHandle {
            panel,
            selected,
            cancelled,
        })
    })
}

/// Presents a save panel.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_save_panel_show(handle: *mut SavePanelHandle) -> i32 {
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        handle.panel.show();
        Ok(())
    })
}

/// Returns whether a save panel is visible.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_save_panel_is_visible(handle: *const SavePanelHandle) -> u8 {
    unsafe { handle.as_ref() }.is_some_and(|handle| handle.panel.is_visible()) as u8
}

/// Moves the latest save-panel selection into the UTF-8 result buffer.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_save_panel_take_selection(handle: *mut SavePanelHandle) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        let selected = handle.selected.borrow_mut().take();
        remember_string(selected.as_deref());
        Ok(())
    })
}

/// Returns and clears the save panel's cancellation flag.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_save_panel_take_cancelled(handle: *mut SavePanelHandle) -> u8 {
    unsafe { handle.as_ref() }.is_some_and(|handle| handle.cancelled.replace(false)) as u8
}

/// Destroys a save panel.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_save_panel_destroy(handle: *mut SavePanelHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Owned recovery store used by the Kome wrapper.
pub struct RecoveryStoreHandle(crate::RecoveryStore);

/// Owned recovery record returned to the Kome wrapper.
pub struct RecoveryRecordHandle(crate::RecoveryRecord);

/// Creates a recovery store rooted at a UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_store_create_utf8(
    data: *const u8,
    length: usize,
) -> *mut RecoveryStoreHandle {
    ffi_pointer(|| {
        let directory = unsafe {
            string(StringView {
                data,
                length: length as u64,
            })?
        };
        Ok(RecoveryStoreHandle(crate::RecoveryStore::new(directory)))
    })
}

/// Saves a UTF-8 recovery record atomically.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_store_save_utf8(
    handle: *mut RecoveryStoreHandle,
    identifier_data: *const u8,
    identifier_length: usize,
    original_path_data: *const u8,
    original_path_length: usize,
    content_type_data: *const u8,
    content_type_length: usize,
    revision: u64,
    contents_data: *const u8,
    contents_length: usize,
) -> i32 {
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        let identifier = unsafe {
            string(StringView {
                data: identifier_data,
                length: identifier_length as u64,
            })?
        };
        let original_path = unsafe {
            string(StringView {
                data: original_path_data,
                length: original_path_length as u64,
            })?
        };
        let content_type = unsafe {
            string(StringView {
                data: content_type_data,
                length: content_type_length as u64,
            })?
        };
        let contents = unsafe { slice::from_raw_parts(contents_data, contents_length) };
        handle
            .0
            .save(&crate::RecoveryRecord {
                identifier: identifier.to_owned(),
                original_path: (!original_path.is_empty()).then(|| PathBuf::from(original_path)),
                content_type: content_type.to_owned(),
                revision,
                contents: contents.to_vec(),
            })
            .map_err(map_io_error)
    })
}

/// Loads a recovery record, returning null when it does not exist.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_store_load_utf8(
    handle: *mut RecoveryStoreHandle,
    identifier_data: *const u8,
    identifier_length: usize,
) -> *mut RecoveryRecordHandle {
    LAST_RESULT_PRESENT.with(|slot| slot.set(0));
    let operation = || -> Result<Option<crate::RecoveryRecord>, Status> {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        let identifier = unsafe {
            string(StringView {
                data: identifier_data,
                length: identifier_length as u64,
            })?
        };
        handle.0.load(identifier).map_err(map_io_error)
    };
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(Some(record))) => {
            LAST_STATUS.with(|slot| slot.set(Status::Ok as i32));
            LAST_RESULT_PRESENT.with(|slot| slot.set(1));
            Box::into_raw(Box::new(RecoveryRecordHandle(record)))
        }
        Ok(Ok(None)) => {
            LAST_STATUS.with(|slot| slot.set(Status::Ok as i32));
            ptr::null_mut()
        }
        Ok(Err(status)) => {
            LAST_STATUS.with(|slot| slot.set(status as i32));
            ptr::null_mut()
        }
        Err(_) => {
            LAST_STATUS.with(|slot| slot.set(Status::Panic as i32));
            ptr::null_mut()
        }
    }
}

/// Removes a recovery record and stores whether it existed as the integer result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_store_remove_utf8(
    handle: *mut RecoveryStoreHandle,
    identifier_data: *const u8,
    identifier_length: usize,
) -> i32 {
    remember_u64(0);
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        let identifier = unsafe {
            string(StringView {
                data: identifier_data,
                length: identifier_length as u64,
            })?
        };
        let removed = handle.0.remove(identifier).map_err(map_io_error)?;
        remember_u64(removed as u64);
        Ok(())
    })
}

/// Stores a recovery record identifier in the UTF-8 result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_record_identifier(
    handle: *const RecoveryRecordHandle,
) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        remember_string(Some(&handle.0.identifier));
        Ok(())
    })
}

/// Stores a recovery record's optional original path in the UTF-8 result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_record_original_path(
    handle: *const RecoveryRecordHandle,
) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        let path = handle
            .0
            .original_path
            .as_deref()
            .and_then(std::path::Path::to_str);
        remember_string(path);
        Ok(())
    })
}

/// Stores a recovery record's content type in the UTF-8 result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_record_content_type(
    handle: *const RecoveryRecordHandle,
) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        remember_string(Some(&handle.0.content_type));
        Ok(())
    })
}

/// Returns a recovery record's revision.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_record_revision(
    handle: *const RecoveryRecordHandle,
) -> u64 {
    unsafe { handle.as_ref() }.map_or(0, |handle| handle.0.revision)
}

/// Stores UTF-8 recovery contents in the result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_record_text_contents(
    handle: *const RecoveryRecordHandle,
) -> i32 {
    remember_string(None);
    ffi_status(|| {
        let handle = unsafe { handle.as_ref() }.ok_or(Status::NullPointer)?;
        let contents = str::from_utf8(&handle.0.contents).map_err(|_| Status::InvalidUtf8)?;
        remember_string(Some(contents));
        Ok(())
    })
}

/// Destroys a recovery record.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_record_destroy(handle: *mut RecoveryRecordHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Destroys a recovery store.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_recovery_store_destroy(handle: *mut RecoveryStoreHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Owned session store used by Kome.
pub struct SessionStoreHandle(crate::SessionStore);

/// Owned application session used by Kome.
pub struct ApplicationSessionHandle(crate::ApplicationSession);

/// Native undo manager exposed through the stable C ABI.
pub struct UndoManagerHandle {
    manager: crate::UndoManager,
    actions: Rc<RefCell<VecDeque<u64>>>,
}

/// Creates an empty undo manager.
#[unsafe(no_mangle)]
pub extern "C" fn mochios_undo_manager_create() -> *mut UndoManagerHandle {
    ffi_pointer(|| {
        Ok(UndoManagerHandle {
            manager: crate::UndoManager::new(),
            actions: Rc::new(RefCell::new(VecDeque::new())),
        })
    })
}

/// Starts an atomic undo group.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_begin_group_utf8(
    manager: *mut UndoManagerHandle,
    name_data: *const u8,
    name_length: usize,
) -> i32 {
    ffi_status(|| {
        let manager = unsafe { manager.as_ref() }.ok_or(Status::NullPointer)?;
        let name = unsafe {
            string(StringView {
                data: name_data,
                length: name_length as u64,
            })?
        };
        manager.manager.begin_group(name);
        Ok(())
    })
}

/// Finishes the current undo group and remembers whether one was open.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_end_group(manager: *mut UndoManagerHandle) -> i32 {
    ffi_status(|| {
        let manager = unsafe { manager.as_ref() }.ok_or(Status::NullPointer)?;
        remember_u64(manager.manager.end_group() as u64);
        Ok(())
    })
}

/// Changes the name of the currently open undo group.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_set_action_name_utf8(
    manager: *mut UndoManagerHandle,
    name_data: *const u8,
    name_length: usize,
) -> i32 {
    ffi_status(|| {
        let manager = unsafe { manager.as_ref() }.ok_or(Status::NullPointer)?;
        let name = unsafe {
            string(StringView {
                data: name_data,
                length: name_length as u64,
            })?
        };
        manager.manager.set_action_name(name);
        Ok(())
    })
}

/// Registers one reversible action using application-owned action tokens.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_register_utf8(
    manager: *mut UndoManagerHandle,
    name_data: *const u8,
    name_length: usize,
    undo_action: u64,
    redo_action: u64,
) -> i32 {
    ffi_status(|| {
        let manager = unsafe { manager.as_ref() }.ok_or(Status::NullPointer)?;
        let name = unsafe {
            string(StringView {
                data: name_data,
                length: name_length as u64,
            })?
        };
        let undo_actions = Rc::clone(&manager.actions);
        let redo_actions = Rc::clone(&manager.actions);
        manager.manager.register(
            name,
            move || undo_actions.borrow_mut().push_back(undo_action),
            move || redo_actions.borrow_mut().push_back(redo_action),
        );
        Ok(())
    })
}

/// Returns whether an undo operation is available.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_can_undo(manager: *const UndoManagerHandle) -> u8 {
    unsafe { manager.as_ref() }.is_some_and(|manager| manager.manager.can_undo()) as u8
}

/// Returns whether a redo operation is available.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_can_redo(manager: *const UndoManagerHandle) -> u8 {
    unsafe { manager.as_ref() }.is_some_and(|manager| manager.manager.can_redo()) as u8
}

/// Stores the current undo action name in the thread-local result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_undo_action_name(
    manager: *const UndoManagerHandle,
) -> i32 {
    ffi_status(|| {
        let manager = unsafe { manager.as_ref() }.ok_or(Status::NullPointer)?;
        remember_string(manager.manager.undo_action_name().as_deref());
        Ok(())
    })
}

/// Stores the current redo action name in the thread-local result buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_redo_action_name(
    manager: *const UndoManagerHandle,
) -> i32 {
    ffi_status(|| {
        let manager = unsafe { manager.as_ref() }.ok_or(Status::NullPointer)?;
        remember_string(manager.manager.redo_action_name().as_deref());
        Ok(())
    })
}

/// Performs the latest undo operation and remembers whether it ran.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_undo(manager: *mut UndoManagerHandle) -> i32 {
    ffi_status(|| {
        let manager = unsafe { manager.as_ref() }.ok_or(Status::NullPointer)?;
        remember_u64(manager.manager.undo() as u64);
        Ok(())
    })
}

/// Performs the latest redo operation and remembers whether it ran.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_redo(manager: *mut UndoManagerHandle) -> i32 {
    ffi_status(|| {
        let manager = unsafe { manager.as_ref() }.ok_or(Status::NullPointer)?;
        remember_u64(manager.manager.redo() as u64);
        Ok(())
    })
}

/// Removes all undo and redo history.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_remove_all(manager: *mut UndoManagerHandle) -> i32 {
    ffi_status(|| {
        unsafe { manager.as_ref() }
            .ok_or(Status::NullPointer)?
            .manager
            .remove_all_actions();
        Ok(())
    })
}

/// Sets the maximum number of committed undo groups.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_set_levels(
    manager: *mut UndoManagerHandle,
    levels: usize,
) -> i32 {
    ffi_status(|| {
        unsafe { manager.as_ref() }
            .ok_or(Status::NullPointer)?
            .manager
            .set_levels_of_undo(levels);
        Ok(())
    })
}

/// Returns the current nested undo grouping level.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_grouping_level(
    manager: *const UndoManagerHandle,
) -> usize {
    unsafe { manager.as_ref() }.map_or(0, |manager| manager.manager.grouping_level())
}

/// Takes the next action token emitted by undo or redo.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_take_action(manager: *mut UndoManagerHandle) -> i32 {
    ffi_status(|| {
        let manager = unsafe { manager.as_ref() }.ok_or(Status::NullPointer)?;
        let action = manager.actions.borrow_mut().pop_front();
        LAST_RESULT_PRESENT.with(|slot| slot.set(action.is_some() as u8));
        remember_u64(action.unwrap_or_default());
        Ok(())
    })
}

/// Destroys an undo manager.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_undo_manager_destroy(manager: *mut UndoManagerHandle) {
    if !manager.is_null() {
        drop(unsafe { Box::from_raw(manager) });
    }
}

/// Creates a session store for a UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_session_store_create_utf8(
    data: *const u8,
    length: usize,
) -> *mut SessionStoreHandle {
    ffi_pointer(|| {
        Ok(SessionStoreHandle(crate::SessionStore::new(unsafe {
            string(StringView {
                data,
                length: length as u64,
            })?
        })))
    })
}

/// Creates an empty application session.
#[unsafe(no_mangle)]
pub extern "C" fn mochios_application_session_create() -> *mut ApplicationSessionHandle {
    ffi_pointer(|| {
        Ok(ApplicationSessionHandle(crate::ApplicationSession {
            windows: Vec::new(),
        }))
    })
}

/// Appends one restorable window to a session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_application_session_add_window_utf8(
    session: *mut ApplicationSessionHandle,
    identifier_data: *const u8,
    identifier_length: usize,
    path_data: *const u8,
    path_length: usize,
    recovery_data: *const u8,
    recovery_length: usize,
    has_frame: u8,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    maximized: u8,
    fullscreen: u8,
) -> i32 {
    ffi_status(|| {
        let session = unsafe { session.as_mut() }.ok_or(Status::NullPointer)?;
        let identifier = unsafe {
            string(StringView {
                data: identifier_data,
                length: identifier_length as u64,
            })?
        };
        let path = unsafe {
            string(StringView {
                data: path_data,
                length: path_length as u64,
            })?
        };
        let recovery = unsafe {
            string(StringView {
                data: recovery_data,
                length: recovery_length as u64,
            })?
        };
        session.0.windows.push(crate::RestorableWindow {
            identifier: identifier.to_owned(),
            document_path: (!path.is_empty()).then(|| PathBuf::from(path)),
            recovery_identifier: (!recovery.is_empty()).then(|| recovery.to_owned()),
            frame: (has_frame != 0).then(|| crate::WindowFrame::new(x, y, width, height)),
            maximized: maximized != 0,
            fullscreen: fullscreen != 0,
        });
        Ok(())
    })
}

/// Saves an application session atomically.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_session_store_save(
    store: *mut SessionStoreHandle,
    session: *const ApplicationSessionHandle,
) -> i32 {
    ffi_status(|| {
        unsafe { store.as_ref() }
            .ok_or(Status::NullPointer)?
            .0
            .save(&unsafe { session.as_ref() }.ok_or(Status::NullPointer)?.0)
            .map_err(map_io_error)
    })
}

/// Loads an application session and records whether it exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_session_store_load(
    store: *mut SessionStoreHandle,
) -> *mut ApplicationSessionHandle {
    LAST_RESULT_PRESENT.with(|slot| slot.set(0));
    let Some(store) = (unsafe { store.as_ref() }) else {
        LAST_STATUS.with(|slot| slot.set(Status::NullPointer as i32));
        return ptr::null_mut();
    };
    match catch_unwind(AssertUnwindSafe(|| store.0.load())) {
        Ok(Ok(Some(session))) => {
            LAST_STATUS.with(|slot| slot.set(0));
            LAST_RESULT_PRESENT.with(|slot| slot.set(1));
            Box::into_raw(Box::new(ApplicationSessionHandle(session)))
        }
        Ok(Ok(None)) => {
            LAST_STATUS.with(|slot| slot.set(0));
            ptr::null_mut()
        }
        Ok(Err(error)) => {
            LAST_STATUS.with(|slot| slot.set(map_io_error(error) as i32));
            ptr::null_mut()
        }
        Err(_) => {
            LAST_STATUS.with(|slot| slot.set(Status::Panic as i32));
            ptr::null_mut()
        }
    }
}

/// Returns the number of windows in a session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_application_session_window_count(
    session: *const ApplicationSessionHandle,
) -> usize {
    unsafe { session.as_ref() }.map_or(0, |session| session.0.windows.len())
}

/// Clears persisted session state and stores whether it existed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_session_store_clear(store: *mut SessionStoreHandle) -> i32 {
    remember_u64(0);
    ffi_status(|| {
        let removed = unsafe { store.as_ref() }
            .ok_or(Status::NullPointer)?
            .0
            .clear()
            .map_err(map_io_error)?;
        remember_u64(removed as u64);
        Ok(())
    })
}

/// Destroys an application session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_application_session_destroy(
    session: *mut ApplicationSessionHandle,
) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

/// Destroys a session store.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_session_store_destroy(store: *mut SessionStoreHandle) {
    if !store.is_null() {
        drop(unsafe { Box::from_raw(store) });
    }
}

#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub extern "C" fn mochios_application_request_exit() -> i32 {
    ffi_status(|| {
        crate::request_exit();
        Ok(())
    })
}

/// Requests that the key ViewKit window close normally.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub extern "C" fn mochios_application_request_close_key_window() -> i32 {
    ffi_status(|| {
        crate::request_close_key_window();
        Ok(())
    })
}

/// Requests application-wide termination.
#[cfg(feature = "ui")]
#[unsafe(no_mangle)]
pub extern "C" fn mochios_application_request_quit() -> i32 {
    ffi_status(|| {
        crate::request_quit();
        Ok(())
    })
}

#[cfg(not(feature = "ui"))]
#[unsafe(no_mangle)]
pub extern "C" fn mochios_application_request_exit() -> i32 {
    Status::UnsupportedPlatform as i32
}

/// Reports that key-window closure is unavailable without ViewKit.
#[cfg(not(feature = "ui"))]
#[unsafe(no_mangle)]
pub extern "C" fn mochios_application_request_close_key_window() -> i32 {
    Status::UnsupportedPlatform as i32
}

/// Reports that application-wide termination is unavailable without ViewKit.
#[cfg(not(feature = "ui"))]
#[unsafe(no_mangle)]
pub extern "C" fn mochios_application_request_quit() -> i32 {
    Status::UnsupportedPlatform as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_has_stable_encoding() {
        assert_eq!(ABI_VERSION, 0x0001_0000);
        assert_eq!(mochios_abi_version(), ABI_VERSION);
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
                mochios_clipboard_set_text(StringView {
                    data: invalid.as_ptr(),
                    length: 1,
                })
            },
            Status::InvalidUtf8 as i32
        );
        assert_eq!(
            unsafe {
                mochios_association_remove(
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

    #[test]
    fn pointer_length_clipboard_entry_rejects_invalid_utf8() {
        let invalid = [0xff];
        assert_eq!(
            unsafe { mochios_clipboard_set_text_utf8(invalid.as_ptr(), invalid.len()) },
            Status::InvalidUtf8 as i32
        );
    }

    #[test]
    fn undo_manager_emits_application_action_tokens() {
        let manager = mochios_undo_manager_create();
        assert!(!manager.is_null());
        assert_eq!(
            unsafe { mochios_undo_manager_register_utf8(manager, b"Typing".as_ptr(), 6, 41, 42) },
            Status::Ok as i32
        );
        assert_eq!(unsafe { mochios_undo_manager_can_undo(manager) }, 1);

        assert_eq!(unsafe { mochios_undo_manager_undo(manager) }, 0);
        assert_eq!(mochios_last_result_u64(), 1);
        assert_eq!(unsafe { mochios_undo_manager_take_action(manager) }, 0);
        assert_eq!(mochios_last_result_has_value(), 1);
        assert_eq!(mochios_last_result_u64(), 41);

        assert_eq!(unsafe { mochios_undo_manager_redo(manager) }, 0);
        assert_eq!(unsafe { mochios_undo_manager_take_action(manager) }, 0);
        assert_eq!(mochios_last_result_u64(), 42);
        unsafe { mochios_undo_manager_destroy(manager) };
    }
}
