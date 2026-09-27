//! Standard application-modal alerts.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use viewkit::event::{EventContext, EventResult, ViewEvent};
use viewkit::platform::Key;
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

struct AlertState {
    visible: Cell<bool>,
    title: RefCell<String>,
    message: RefCell<String>,
}

/// A standard application-modal error alert.
///
/// Present the alert from an operation callback, then include it in the owning
/// view's paint and event paths while [`is_visible`](Self::is_visible) is true.
#[derive(Clone)]
pub struct Alert {
    state: Rc<AlertState>,
    dismiss: Rc<Button>,
}

impl Alert {
    #[must_use]
    pub fn new() -> Self {
        let state = Rc::new(AlertState {
            visible: Cell::new(false),
            title: RefCell::new(String::new()),
            message: RefCell::new(String::new()),
        });
        let dismiss_state = Rc::clone(&state);
        let dismiss = Rc::new(
            Button::new("OK")
                .size(ButtonSize::Small)
                .style(ButtonStyle::Accent)
                .on_click(move || dismiss_state.visible.set(false)),
        );
        Self { state, dismiss }
    }

    /// Presents an error and replaces any error already being shown.
    pub fn present_error(&self, title: impl Into<String>, message: impl Into<String>) {
        *self.state.title.borrow_mut() = title.into();
        *self.state.message.borrow_mut() = message.into();
        self.state.visible.set(true);
    }

    pub fn dismiss(&self) {
        self.state.visible.set(false);
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.state.visible.get()
    }

    fn geometry(bounds: Rect) -> (Rect, Rect, Rect, Rect) {
        let width = (bounds.size.width - 64.0).clamp(380.0, 520.0);
        let height = 188.0_f32.min((bounds.size.height - 32.0).max(0.0));
        let dialog = Rect::new(
            bounds.origin.x + (bounds.size.width - width) / 2.0,
            bounds.origin.y + (bounds.size.height - height) / 2.0,
            width,
            height,
        );
        let icon = Rect::new(dialog.origin.x + 22.0, dialog.origin.y + 24.0, 40.0, 40.0);
        let text = Rect::new(
            icon.origin.x + icon.size.width + 16.0,
            dialog.origin.y + 22.0,
            (dialog.size.width - 100.0).max(0.0),
            92.0,
        );
        let button = Rect::new(
            dialog.origin.x + dialog.size.width - 100.0,
            dialog.origin.y + dialog.size.height - 52.0,
            78.0,
            30.0,
        );
        (dialog, icon, text, button)
    }
}

impl Default for Alert {
    fn default() -> Self {
        Self::new()
    }
}

impl View for Alert {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(constraints.maximum)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        if !self.is_visible() {
            return;
        }
        let (dialog, icon, text, button) = Self::geometry(bounds);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.shell.scrim))
            .paint(bounds, context);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.dialog.background))
            .radius(context.theme.dialog.radius)
            .border(BorderStyle::custom(
                context.theme.dialog.border,
                context.theme.dialog.stroke_width,
            ))
            .paint(dialog, context);

        let mut node = AccessibilityNode::new(AccessibilityRole::Dialog, dialog);
        node.label = Some(self.state.title.borrow().clone());
        node.value = Some(self.state.message.borrow().clone());
        node.focus_scope = true;
        node.invalid = true;
        context.record_accessibility(node);

        Ellipse::new()
            .color(EllipseColor::Custom(context.theme.shell.alert))
            .paint(icon, context);
        Text::new("!")
            .accessibility_hidden(true)
            .font_size(25.0)
            .line_height(icon.size.height)
            .weight(750)
            .alignment(TextAlignment::Center)
            .color(context.theme.shell.inverse_text)
            .paint(icon, context);

        Text::styled(self.state.title.borrow().clone(), TextRole::TitleSmall).paint(
            Rect::new(text.origin.x, text.origin.y, text.size.width, 26.0),
            context,
        );
        Text::body(self.state.message.borrow().clone()).paint(
            Rect::new(text.origin.x, text.origin.y + 34.0, text.size.width, 58.0),
            context,
        );
        self.dismiss.paint(button, context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if !self.is_visible() {
            return EventResult::Ignored;
        }
        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Enter | Key::Escape,
                ..
            }
        ) {
            self.dismiss();
            context.request_redraw();
            return EventResult::Consumed;
        }
        let (_, _, _, button) = Self::geometry(bounds);
        let result = self.dismiss.handle_event(button, event, context);
        if result.is_consumed() {
            context.request_redraw();
        }
        EventResult::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presenting_replaces_the_current_error() {
        let alert = Alert::new();
        alert.present_error("First", "One");
        alert.present_error("Second", "Two");
        assert!(alert.is_visible());
        assert_eq!(alert.state.title.borrow().as_str(), "Second");
        assert_eq!(alert.state.message.borrow().as_str(), "Two");
        alert.dismiss();
        assert!(!alert.is_visible());
    }
}
