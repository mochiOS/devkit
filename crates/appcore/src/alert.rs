//! Standard application-modal alert windows.

use std::cell::Cell;
use std::rc::Rc;

use viewkit::app::WindowId;
use viewkit::event::{EventContext, EventResult, ViewEvent};
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

struct AlertState {
    window: Cell<Option<WindowId>>,
}

/// A standard, frontmost error window owned by the application.
///
/// ViewKit renders the alert in its own native top-level window, so an
/// application's root `View` type does not need to provide a separate body for
/// it. The `View` implementation remains intentionally empty for compatibility
/// with containers such as `DocumentController`.
#[derive(Clone)]
pub struct Alert {
    state: Rc<AlertState>,
}

impl Alert {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Rc::new(AlertState {
                window: Cell::new(None),
            }),
        }
    }

    /// Presents an error in a new frontmost window, replacing an existing one.
    pub fn present_error(&self, title: impl Into<String>, message: impl Into<String>) {
        if let Some(window) = self.state.window.get() {
            viewkit::close_window(window);
        }

        let state = Rc::clone(&self.state);
        let requested = Rc::new(Cell::new(None));
        let dismissed = Rc::clone(&requested);
        let window = viewkit::request_alert_window(title, message, move || {
            if state.window.get() == dismissed.get() {
                state.window.set(None);
            }
        });
        requested.set(Some(window));
        self.state.window.set(Some(window));
    }

    pub fn dismiss(&self) {
        if let Some(window) = self.state.window.take() {
            viewkit::close_window(window);
        }
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.state.window.get().is_some()
    }
}

impl Default for Alert {
    fn default() -> Self {
        Self::new()
    }
}

impl View for Alert {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(Size::new(0.0, 0.0))
    }

    fn paint(&self, _bounds: Rect, _context: &mut PaintContext<'_>) {}

    fn handle_event(
        &self,
        _bounds: Rect,
        _event: &ViewEvent,
        _context: &mut EventContext<'_>,
    ) -> EventResult {
        EventResult::Ignored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presenting_replaces_the_current_error_window() {
        let alert = Alert::new();
        alert.present_error("First", "One");
        let first = alert.state.window.get().unwrap();
        alert.present_error("Second", "Two");
        let second = alert.state.window.get().unwrap();
        assert_ne!(first, second);
        assert!(alert.is_visible());
        alert.dismiss();
        assert!(!alert.is_visible());
    }
}
