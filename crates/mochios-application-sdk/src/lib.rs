//! The supported application-development API for mochiOS.
//!
//! This crate is the stable entry point for application code. ViewKit remains
//! the UI implementation, while OS integration is exposed through small,
//! capability-checked modules. Applications still need to declare the matching
//! capabilities in their manifest; using this SDK never bypasses system policy.

pub mod clipboard;
pub mod document;
pub mod error;
pub mod ffi;

pub use error::{Error, Result};

#[cfg(feature = "ui")]
pub use viewkit;
#[cfg(feature = "ui")]
pub use viewkit::{ViewKitError, run};

/// Common imports for a Rust mochiOS application.
pub mod prelude {
    pub use crate::clipboard;
    pub use crate::document::{self, AssociationHandler, AssociationRoles};
    pub use crate::error::{Error as ApplicationError, Result as ApplicationResult};

    #[cfg(feature = "ui")]
    pub use viewkit::prelude::*;
}

/// Requests that the current ViewKit application terminate cleanly.
#[cfg(feature = "ui")]
pub fn request_exit() {
    viewkit::request_exit();
}
