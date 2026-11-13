//! Window creation and input-authorized operations using the shared ViewKit
//! runtime. System controls remain owned by the window system.

/// Window options consumed by `App::window` and `App::window_for`.
pub use viewkit::app::{WindowId, WindowOptions};
/// Ordinary ViewKit components used inside an integrated title bar.
pub use viewkit::components::{DragRegion, TitleBar, drag_region};
/// Decoration settings, errors and input-authorized resize edges.
pub use viewkit::decoration::{
    DecorationError, DecorationLayout, DecorationMode, ResizeEdge, WindowInteractionToken,
    WindowState, metrics,
};

/// Opens another application window using `App::window_for` and `App::body_for`.
/// The returned identity remains stable until that window closes.
pub fn open_window() -> WindowId {
    viewkit::runtime::request_new_window()
}

/// Begins moving the window which issued this event token. Call only from the
/// input handler; a token cannot be constructed or saved beyond event dispatch.
/// Returns false if another handler already consumed this press.
pub fn begin_move(token: WindowInteractionToken<'_>) -> bool {
    viewkit::runtime::begin_window_interaction(token, viewkit::decoration::WindowInteraction::Move)
}

/// Begins resizing one or two edges of the token's window. The compositor
/// checks ownership, the active primary press and the resizable attribute.
pub fn begin_resize(token: WindowInteractionToken<'_>, edge: ResizeEdge) -> bool {
    viewkit::runtime::begin_window_interaction(
        token,
        viewkit::decoration::WindowInteraction::Resize(edge),
    )
}
