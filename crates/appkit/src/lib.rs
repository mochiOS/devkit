//! AppKit is the supported application framework for mochiOS.
//!
//! This crate is the stable entry point for application code. ViewKit remains
//! the UI implementation, while OS integration is exposed through small,
//! capability-checked modules. Applications still need to declare the matching
//! capabilities in their manifest; using this SDK never bypasses system policy.

pub mod clipboard;
pub mod content_type;
pub mod document;
#[cfg(feature = "ui")]
pub mod document_controller;
pub mod error;
pub mod ffi;
#[cfg(feature = "ui")]
pub mod menu;
#[cfg(feature = "ui")]
pub mod panel;
pub mod recovery;
pub mod session;
pub mod undo;
#[cfg(feature = "ui")]
pub mod undo_responder;

pub use error::{Error, Result};
pub use recovery::{RecoveryRecord, RecoveryStore};
pub use session::{ApplicationSession, RestorableWindow, SessionStore, WindowFrame};
pub use undo::UndoManager;
#[cfg(feature = "ui")]
pub use undo_responder::UndoResponder;

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
    pub use crate::content_type::{self, ContentType};
    pub use crate::document::{self, AssociationHandler, AssociationRoles};
    pub use crate::error::{Error as ApplicationError, Result as ApplicationResult};
    pub use crate::recovery::{RecoveryRecord, RecoveryStore};
    pub use crate::session::{ApplicationSession, RestorableWindow, SessionStore, WindowFrame};
    pub use crate::undo::UndoManager;
    #[cfg(feature = "ui")]
    pub use crate::undo_responder::UndoResponder;

    #[cfg(feature = "ui")]
    pub use crate::document_controller::{DocumentController, DocumentInfo, DocumentMetadata};
    #[cfg(feature = "ui")]
    pub use crate::menu::{ApplicationMenu, ApplicationMenuBar, ApplicationMenuItem, MenuShortcut};
    #[cfg(feature = "ui")]
    pub use crate::panel::{OpenPanel, OpenPanelOptions, SavePanel, SavePanelOptions};
    #[cfg(feature = "ui")]
    pub use crate::{request_close_key_window, request_quit};
    #[cfg(feature = "ui")]
    pub use viewkit::prelude::*;
}

/// Requests that the current ViewKit application terminate cleanly.
#[cfg(feature = "ui")]
pub fn request_exit() {
    viewkit::request_exit();
}

/// Requests that the key window close after normal document confirmation.
#[cfg(feature = "ui")]
pub fn request_close_key_window() {
    viewkit::request_close_key_window();
}

/// Requests application-wide termination.
#[cfg(feature = "ui")]
pub fn request_quit() {
    viewkit::request_exit();
}
