//! AppKit is the supported application framework for mochiOS.
//!
//! This crate is the stable entry point for application code. ViewKit remains
//! the UI implementation, while OS integration is exposed through small,
//! capability-checked modules. Applications still need to declare the matching
//! capabilities in their manifest; using this SDK never bypasses system policy.

pub mod clipboard;
pub mod document;
#[cfg(feature = "ui")]
pub mod document_controller;
pub mod error;
pub mod ffi;
#[cfg(feature = "ui")]
pub mod menu;
#[cfg(feature = "ui")]
pub mod panel;

pub use error::{Error, Result};

#[cfg(feature = "ui")]
pub use document_controller::{DocumentController, DocumentInfo, DocumentMetadata};

#[cfg(feature = "ui")]
pub use menu::{ApplicationMenu, ApplicationMenuBar, ApplicationMenuItem, MenuShortcut};
#[cfg(feature = "ui")]
pub use panel::{OpenPanel, OpenPanelOptions, SavePanel, SavePanelOptions};
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
    pub use crate::document_controller::{DocumentController, DocumentInfo, DocumentMetadata};
    #[cfg(feature = "ui")]
    pub use crate::menu::{ApplicationMenu, ApplicationMenuBar, ApplicationMenuItem, MenuShortcut};
    #[cfg(feature = "ui")]
    pub use crate::panel::{OpenPanel, OpenPanelOptions, SavePanel, SavePanelOptions};
    #[cfg(feature = "ui")]
    pub use viewkit::prelude::*;
}

/// Requests that the current ViewKit application terminate cleanly.
#[cfg(feature = "ui")]
pub fn request_exit() {
    viewkit::request_exit();
}
