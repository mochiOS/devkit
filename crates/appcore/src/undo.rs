//! Reusable undo and redo transactions for document-based applications.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Action = Box<dyn FnMut()>;

struct Operation {
    undo: Action,
    redo: Action,
}

struct Group {
    name: String,
    operations: Vec<Operation>,
}

#[derive(Default)]
struct State {
    undo: Vec<Group>,
    redo: Vec<Group>,
    pending: Option<Group>,
    grouping_level: usize,
    levels: usize,
}

/// Records reversible application operations in named groups.
///
/// Operations registered between [`begin_group`](Self::begin_group) and
/// [`end_group`](Self::end_group) undo atomically. Without an explicit group,
/// each registration is its own undo step.
#[derive(Clone)]
pub struct UndoManager {
    state: Rc<RefCell<State>>,
    replaying: Rc<Cell<bool>>,
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new()
    }
}

impl UndoManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(State {
                levels: 100,
                ..State::default()
            })),
            replaying: Rc::new(Cell::new(false)),
        }
    }

    /// Starts an atomic undo group. Nested groups collapse into the outer one.
    pub fn begin_group(&self, name: impl Into<String>) {
        let mut state = self.state.borrow_mut();
        if state.grouping_level == 0 {
            state.pending = Some(Group {
                name: name.into(),
                operations: Vec::new(),
            });
        }
        state.grouping_level += 1;
    }

    /// Finishes the current group and makes it available to Undo.
    /// Returns `false` when no group was open.
    pub fn end_group(&self) -> bool {
        let mut state = self.state.borrow_mut();
        if state.grouping_level == 0 {
            return false;
        }
        state.grouping_level -= 1;
        if state.grouping_level != 0 {
            return true;
        }
        if let Some(group) = state.pending.take()
            && !group.operations.is_empty()
        {
            push_undo(&mut state, group);
        }
        true
    }

    /// Changes the title shown for the currently open group.
    pub fn set_action_name(&self, name: impl Into<String>) {
        if let Some(group) = self.state.borrow_mut().pending.as_mut() {
            group.name = name.into();
        }
    }

    /// Registers a reversible operation. The callbacks must describe opposite
    /// transitions over the same model state.
    pub fn register(
        &self,
        name: impl Into<String>,
        undo: impl FnMut() + 'static,
        redo: impl FnMut() + 'static,
    ) {
        if self.replaying.get() {
            return;
        }
        let mut state = self.state.borrow_mut();
        if state.levels == 0 {
            return;
        }
        let operation = Operation {
            undo: Box::new(undo),
            redo: Box::new(redo),
        };
        state.redo.clear();
        if let Some(group) = state.pending.as_mut() {
            group.operations.push(operation);
        } else {
            let group = Group {
                name: name.into(),
                operations: vec![operation],
            };
            push_undo(&mut state, group);
        }
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.state.borrow().undo.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.state.borrow().redo.is_empty()
    }

    #[must_use]
    pub fn undo_action_name(&self) -> Option<String> {
        self.state
            .borrow()
            .undo
            .last()
            .map(|group| group.name.clone())
    }

    #[must_use]
    pub fn redo_action_name(&self) -> Option<String> {
        self.state
            .borrow()
            .redo
            .last()
            .map(|group| group.name.clone())
    }

    pub fn undo(&self) -> bool {
        let Some(mut group) = self.state.borrow_mut().undo.pop() else {
            return false;
        };
        self.replaying.set(true);
        for operation in group.operations.iter_mut().rev() {
            (operation.undo)();
        }
        self.replaying.set(false);
        self.state.borrow_mut().redo.push(group);
        true
    }

    pub fn redo(&self) -> bool {
        let Some(mut group) = self.state.borrow_mut().redo.pop() else {
            return false;
        };
        self.replaying.set(true);
        for operation in &mut group.operations {
            (operation.redo)();
        }
        self.replaying.set(false);
        let mut state = self.state.borrow_mut();
        push_undo(&mut state, group);
        true
    }

    pub fn remove_all_actions(&self) {
        let mut state = self.state.borrow_mut();
        state.undo.clear();
        state.redo.clear();
        state.pending = None;
        state.grouping_level = 0;
    }

    /// Sets the maximum number of committed groups. Zero disables recording.
    pub fn set_levels_of_undo(&self, levels: usize) {
        let mut state = self.state.borrow_mut();
        state.levels = levels;
        trim_undo(&mut state);
        if levels == 0 {
            state.redo.clear();
            state.pending = None;
            state.grouping_level = 0;
        }
    }

    #[must_use]
    pub fn grouping_level(&self) -> usize {
        self.state.borrow().grouping_level
    }
}

fn push_undo(state: &mut State, group: Group) {
    state.undo.push(group);
    trim_undo(state);
}

fn trim_undo(state: &mut State) {
    if state.undo.len() > state.levels {
        state.undo.drain(..state.undo.len() - state.levels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_apply_opposite_transitions() {
        let value = Rc::new(Cell::new(1));
        let undo_value = Rc::clone(&value);
        let redo_value = Rc::clone(&value);
        let manager = UndoManager::new();
        manager.register(
            "Typing",
            move || undo_value.set(0),
            move || redo_value.set(1),
        );

        assert!(manager.undo());
        assert_eq!(value.get(), 0);
        assert!(manager.redo());
        assert_eq!(value.get(), 1);
    }

    #[test]
    fn groups_undo_in_reverse_and_redo_in_forward_order() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let manager = UndoManager::new();
        manager.begin_group("Replace");
        for value in [1, 2] {
            let undo_log = Rc::clone(&log);
            let redo_log = Rc::clone(&log);
            manager.register(
                "ignored",
                move || undo_log.borrow_mut().push(-value),
                move || redo_log.borrow_mut().push(value),
            );
        }
        assert!(manager.end_group());
        assert_eq!(manager.undo_action_name().as_deref(), Some("Replace"));
        assert!(manager.undo());
        assert_eq!(*log.borrow(), vec![-2, -1]);
        assert!(manager.redo());
        assert_eq!(*log.borrow(), vec![-2, -1, 1, 2]);
    }

    #[test]
    fn new_registration_discards_redo_history() {
        let manager = UndoManager::new();
        manager.register("First", || {}, || {});
        assert!(manager.undo());
        assert!(manager.can_redo());
        manager.register("Second", || {}, || {});
        assert!(!manager.can_redo());
    }
}
