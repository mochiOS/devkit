//! Application menu bars and commands.
//!
//! The bar is part of the application's window content. Expanded menus are
//! handed to ViewKit's platform popup path so the window system owns popup
//! placement, focus, pointer capture, and dismissal.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use viewkit::accessibility::{AccessibilityNode, AccessibilityRole};
use viewkit::event::{ContextMenuItem, ContextMenuRequest, EventContext, EventResult, ViewEvent};
use viewkit::platform::{Key, KeyModifiers, PointerButton};
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

type Action = Rc<RefCell<Box<dyn FnMut()>>>;
type Predicate = Rc<dyn Fn() -> bool>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuShortcut {
    key: Key,
    requires_shortcut: bool,
    requires_shift: bool,
    requires_alt: bool,
    label: String,
}

impl MenuShortcut {
    pub fn command(key: char, label: impl Into<String>) -> Self {
        Self {
            key: Key::Character(key.to_ascii_lowercase()),
            requires_shortcut: true,
            requires_shift: false,
            requires_alt: false,
            label: label.into(),
        }
    }

    #[must_use]
    pub fn shift(mut self) -> Self {
        self.requires_shift = true;
        self
    }

    #[must_use]
    pub fn alt(mut self) -> Self {
        self.requires_alt = true;
        self
    }

    fn matches(&self, key: Key, modifiers: KeyModifiers) -> bool {
        let key_matches = match (self.key, key) {
            (Key::Character(expected), Key::Character(actual)) => {
                expected.eq_ignore_ascii_case(&actual)
            }
            (expected, actual) => expected == actual,
        };
        key_matches
            && (!self.requires_shortcut || modifiers.shortcut())
            && modifiers.shift() == self.requires_shift
            && modifiers.alt() == self.requires_alt
    }
}

pub struct ApplicationMenuItem {
    label: String,
    shortcut: Option<MenuShortcut>,
    enabled: Predicate,
    checked: Predicate,
    destructive: bool,
    action: Action,
}

impl ApplicationMenuItem {
    pub fn new(label: impl Into<String>, action: impl FnMut() + 'static) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            enabled: Rc::new(|| true),
            checked: Rc::new(|| false),
            destructive: false,
            action: Rc::new(RefCell::new(Box::new(action))),
        }
    }

    #[must_use]
    pub fn shortcut(mut self, shortcut: MenuShortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = Rc::new(move || enabled);
        self
    }

    #[must_use]
    pub fn enabled_when(mut self, predicate: impl Fn() -> bool + 'static) -> Self {
        self.enabled = Rc::new(predicate);
        self
    }

    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Rc::new(move || checked);
        self
    }

    #[must_use]
    pub fn checked_when(mut self, predicate: impl Fn() -> bool + 'static) -> Self {
        self.checked = Rc::new(predicate);
        self
    }

    #[must_use]
    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    fn invoke(&self) -> bool {
        if !(self.enabled)() {
            return false;
        }
        (self.action.borrow_mut())();
        true
    }

    fn popup_label(&self) -> String {
        match self.shortcut.as_ref() {
            Some(shortcut) => format!("{}    {}", self.label, shortcut.label),
            None => self.label.clone(),
        }
    }
}

enum ApplicationMenuEntry {
    Item(ApplicationMenuItem),
    Separator,
}

pub struct ApplicationMenu {
    title: String,
    entries: Vec<ApplicationMenuEntry>,
}

impl ApplicationMenu {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            entries: Vec::new(),
        }
    }

    #[must_use]
    pub fn item(mut self, item: ApplicationMenuItem) -> Self {
        self.entries.push(ApplicationMenuEntry::Item(item));
        self
    }

    #[must_use]
    pub fn separator(mut self) -> Self {
        self.entries.push(ApplicationMenuEntry::Separator);
        self
    }

    fn popup_items(&self) -> Vec<ContextMenuItem> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, entry)| match entry {
                ApplicationMenuEntry::Item(item) => ContextMenuItem {
                    command_id: index as u32 + 1,
                    label: item.popup_label(),
                    enabled: (item.enabled)(),
                    checked: (item.checked)(),
                    destructive: item.destructive,
                    separator: false,
                },
                ApplicationMenuEntry::Separator => ContextMenuItem {
                    command_id: 0,
                    label: String::new(),
                    enabled: false,
                    checked: false,
                    destructive: false,
                    separator: true,
                },
            })
            .collect()
    }

    fn invoke(&self, command_id: u32) -> bool {
        let Some(index) = command_id.checked_sub(1).map(|index| index as usize) else {
            return false;
        };
        match self.entries.get(index) {
            Some(ApplicationMenuEntry::Item(item)) => item.invoke(),
            _ => false,
        }
    }

    fn invoke_shortcut(&self, key: Key, modifiers: KeyModifiers) -> bool {
        self.entries.iter().any(|entry| match entry {
            ApplicationMenuEntry::Item(item)
                if item
                    .shortcut
                    .as_ref()
                    .is_some_and(|shortcut| shortcut.matches(key, modifiers)) =>
            {
                item.invoke()
            }
            _ => false,
        })
    }
}

pub struct ApplicationMenuBar<Content> {
    content: Content,
    menus: Vec<ApplicationMenu>,
    pending_request: Cell<Option<(u64, usize)>>,
    hovered_menu: Cell<Option<usize>>,
}

impl<Content> ApplicationMenuBar<Content> {
    pub fn new(content: Content) -> Self {
        Self {
            content,
            menus: Vec::new(),
            pending_request: Cell::new(None),
            hovered_menu: Cell::new(None),
        }
    }

    #[must_use]
    pub fn menu(mut self, menu: ApplicationMenu) -> Self {
        self.menus.push(menu);
        self
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    fn bar_height(theme: &Theme) -> f32 {
        theme.layout.compact_control_height
    }

    fn bar_bounds(bounds: Rect, theme: &Theme) -> Rect {
        Rect::new(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            Self::bar_height(theme).min(bounds.size.height),
        )
    }

    fn content_bounds(bounds: Rect, theme: &Theme) -> Rect {
        let height = Self::bar_height(theme).min(bounds.size.height);
        Rect::new(
            bounds.origin.x,
            bounds.origin.y + height,
            bounds.size.width,
            (bounds.size.height - height).max(0.0),
        )
    }

    fn menu_width(menu: &ApplicationMenu, theme: &Theme) -> f32 {
        let estimated_text = menu.title.chars().count() as f32 * 8.0;
        (estimated_text + theme.spacing.large * 2.0).max(48.0)
    }

    fn menu_bounds(&self, bounds: Rect, theme: &Theme) -> Vec<Rect> {
        let bar = Self::bar_bounds(bounds, theme);
        let mut x = bar.origin.x + theme.spacing.small;
        self.menus
            .iter()
            .map(|menu| {
                let width = Self::menu_width(menu, theme);
                let result = Rect::new(x, bar.origin.y, width, bar.size.height);
                x += width;
                result
            })
            .collect()
    }

    fn menu_at(&self, bounds: Rect, position: Point, theme: &Theme) -> Option<usize> {
        self.menu_bounds(bounds, theme)
            .iter()
            .position(|menu| menu.contains(position))
    }

    fn show_menu(&self, index: usize, anchor: Rect, context: &mut EventContext<'_>) {
        let Some(menu) = self.menus.get(index) else {
            return;
        };
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        self.pending_request.set(Some((request_id, index)));
        context.show_context_menu(ContextMenuRequest {
            request_id,
            position: Point::new(anchor.origin.x, anchor.origin.y + anchor.size.height),
            items: menu.popup_items(),
        });
        context.request_redraw();
    }
}

impl<Content: View> View for ApplicationMenuBar<Content> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        let bar_height = Self::bar_height(context.theme);
        let content_constraints = Constraints::new(
            Size::new(
                constraints.minimum.width,
                (constraints.minimum.height - bar_height).max(0.0),
            ),
            Size::new(
                constraints.maximum.width,
                (constraints.maximum.height - bar_height).max(0.0),
            ),
        );
        let content = self.content.measure(content_constraints, context);
        constraints.constrain(Size::new(content.width, content.height + bar_height))
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content
            .paint(Self::content_bounds(bounds, context.theme), context);

        let bar = Self::bar_bounds(bounds, context.theme);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.colors.surface_subtle))
            .paint(bar, context);

        for (index, (menu, menu_bounds)) in self
            .menus
            .iter()
            .zip(self.menu_bounds(bounds, context.theme))
            .enumerate()
        {
            let mut accessibility =
                AccessibilityNode::new(AccessibilityRole::MenuItem, menu_bounds);
            accessibility.label = Some(menu.title.clone());
            accessibility.focusable = true;
            context.record_accessibility(accessibility);
            if self.hovered_menu.get() == Some(index) {
                Rectangle::new()
                    .color(RectangleColor::Custom(
                        context.theme.menu.item_hovered_background,
                    ))
                    .radius(context.theme.menu.item_radius)
                    .paint(menu_bounds, context);
            }
            let line_height = context.typography.style(TextRole::Label).line_height
                * context.text_measurer.font_scale();
            let text_bounds = Rect::new(
                menu_bounds.origin.x,
                menu_bounds.origin.y + ((menu_bounds.size.height - line_height) / 2.0).max(0.0),
                menu_bounds.size.width,
                line_height.min(menu_bounds.size.height),
            );
            Text::label(menu.title.clone())
                .alignment(TextAlignment::Center)
                .paint(text_bounds, context);
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if let ViewEvent::ContextMenuResult {
            request_id,
            command_id,
        } = event
            && self
                .pending_request
                .get()
                .is_some_and(|(pending, _)| pending == *request_id)
        {
            let (_, menu_index) = self.pending_request.take().expect("request was checked");
            if let Some(command_id) = command_id
                && let Some(menu) = self.menus.get(menu_index)
            {
                let _ = menu.invoke(*command_id);
            }
            context.request_redraw();
            return EventResult::Consumed;
        }

        if let ViewEvent::KeyPressed { key, modifiers } = event
            && self
                .menus
                .iter()
                .any(|menu| menu.invoke_shortcut(*key, *modifiers))
        {
            context.request_redraw();
            return EventResult::Consumed;
        }

        let bar = Self::bar_bounds(bounds, context.theme());
        match event {
            ViewEvent::PointerMoved { position } if bar.contains(*position) => {
                let hovered = self.menu_at(bounds, *position, context.theme());
                if self.hovered_menu.replace(hovered) != hovered {
                    context.request_redraw_in(bar);
                }
                return EventResult::Consumed;
            }
            ViewEvent::PointerLeft => {
                if self.hovered_menu.take().is_some() {
                    context.request_redraw_in(bar);
                }
            }
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } if bar.contains(*position) => {
                if let Some(index) = self.menu_at(bounds, *position, context.theme()) {
                    let menu_bounds = self.menu_bounds(bounds, context.theme())[index];
                    self.show_menu(index, menu_bounds, context);
                }
                return EventResult::Consumed;
            }
            _ => {}
        }

        self.content.handle_event(
            Self::content_bounds(bounds, context.theme()),
            event,
            context,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewkit::typography::TextMeasurer;

    #[test]
    fn shortcut_invokes_enabled_item() {
        let calls = Rc::new(Cell::new(0));
        let action_calls = Rc::clone(&calls);
        let menu = ApplicationMenu::new("File").item(
            ApplicationMenuItem::new("Save", move || action_calls.set(action_calls.get() + 1))
                .shortcut(MenuShortcut::command('s', "Ctrl+S")),
        );
        assert!(menu.invoke_shortcut(
            Key::Character('S'),
            KeyModifiers::from_bits(KeyModifiers::CONTROL),
        ));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn disabled_items_do_not_invoke() {
        let calls = Rc::new(Cell::new(0));
        let action_calls = Rc::clone(&calls);
        let item = ApplicationMenuItem::new("Unavailable", move || {
            action_calls.set(action_calls.get() + 1)
        })
        .enabled(false);
        assert!(!item.invoke());
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn menu_bar_reserves_space_above_content() {
        let theme = Theme::LIGHT;
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let content = ApplicationMenuBar::<Text>::content_bounds(bounds, &theme);
        assert_eq!(content.origin.y, theme.layout.compact_control_height);
        assert_eq!(
            content.size.height,
            600.0 - theme.layout.compact_control_height
        );

        let mut text_measurer = TextMeasurer::new();
        let mut context = MeasureContext {
            theme: &theme,
            typography: &theme.typography,
            text_measurer: &mut text_measurer,
        };
        let measured = ApplicationMenuBar::new(Text::body(""))
            .measure(Constraints::loose(bounds.size), &mut context);
        assert!(measured.height >= theme.layout.compact_control_height);
    }

    #[test]
    fn clicking_a_menu_requests_a_popup_and_dispatches_its_command() {
        let calls = Rc::new(Cell::new(0));
        let action_calls = Rc::clone(&calls);
        let bar = ApplicationMenuBar::new(Text::body("Document")).menu(
            ApplicationMenu::new("File").item(ApplicationMenuItem::new("Open", move || {
                action_calls.set(action_calls.get() + 1)
            })),
        );
        let theme = Theme::LIGHT;
        let mut text_measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut text_measurer);
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);

        assert_eq!(
            bar.handle_event(
                bounds,
                &ViewEvent::PointerReleased {
                    position: Point::new(20.0, 10.0),
                    button: PointerButton::Primary,
                },
                &mut context,
            ),
            EventResult::Consumed
        );
        let request = context.take_context_menu_request().unwrap();
        assert_eq!(request.items.len(), 1);
        assert_eq!(request.items[0].label, "Open");

        assert_eq!(
            bar.handle_event(
                bounds,
                &ViewEvent::ContextMenuResult {
                    request_id: request.request_id,
                    command_id: Some(1),
                },
                &mut context,
            ),
            EventResult::Consumed
        );
        assert_eq!(calls.get(), 1);
    }
}
