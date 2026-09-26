//! Responder-chain integration for an [`UndoManager`](crate::UndoManager).

use viewkit::command::CommandStatus;
use viewkit::command::standard as commands;
use viewkit::event::{EventContext, EventResult, ViewEvent};
use viewkit::geometry::{Rect, Size};
use viewkit::view::{Constraints, MeasureContext, PaintContext, View};

use crate::UndoManager;

/// Handles Undo and Redo after the focused child has had first refusal.
pub struct UndoResponder<Content> {
    content: Content,
    manager: UndoManager,
}

impl<Content> UndoResponder<Content> {
    pub fn new(content: Content, manager: UndoManager) -> Self {
        Self { content, manager }
    }

    pub fn manager(&self) -> &UndoManager {
        &self.manager
    }

    pub fn content(&self) -> &Content {
        &self.content
    }
}

impl<Content: View> View for UndoResponder<Content> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        let undo = CommandStatus::new(commands::UNDO, bounds, self.manager.can_undo());
        let undo = match self.manager.undo_action_name() {
            Some(name) => undo.title(format!("Undo {name}")),
            None => undo,
        };
        context.record_command_status(undo);
        let redo = CommandStatus::new(commands::REDO, bounds, self.manager.can_redo());
        let redo = match self.manager.redo_action_name() {
            Some(name) => redo.title(format!("Redo {name}")),
            None => redo,
        };
        context.record_command_status(redo);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let result = self.content.handle_event(bounds, event, context);
        if result.is_consumed() {
            return result;
        }
        let ViewEvent::Command { command, .. } = event else {
            return EventResult::Ignored;
        };
        let handled = if *command == commands::UNDO {
            self.manager.undo()
        } else if *command == commands::REDO {
            self.manager.redo()
        } else {
            return EventResult::Ignored;
        };
        if handled {
            context.request_redraw();
        }
        EventResult::Consumed
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use viewkit::components::Text;
    use viewkit::theme::Theme;
    use viewkit::typography::TextMeasurer;

    #[test]
    fn handles_undo_when_the_child_does_not() {
        let value = Rc::new(Cell::new(1));
        let undo_value = Rc::clone(&value);
        let manager = UndoManager::new();
        manager.register("Change", move || undo_value.set(0), || {});
        let responder = UndoResponder::new(Text::body(""), manager);
        let theme = Theme::LIGHT;
        let mut text_measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut text_measurer);

        assert_eq!(
            responder.handle_event(
                Rect::new(0.0, 0.0, 100.0, 100.0),
                &ViewEvent::Command {
                    command: commands::UNDO,
                    target: None,
                },
                &mut context,
            ),
            EventResult::Consumed
        );
        assert_eq!(value.get(), 0);
    }
}
