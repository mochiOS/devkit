//! Standard file panels for mochiOS applications.
//!
//! `SavePanel` and `OpenPanel` own navigation, path validation, replacement
//! confirmation, and presentation. Applications provide only the operation to
//! perform after the user has selected a valid path.

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use viewkit::draw_command::DrawCommand;
use viewkit::event::{EventContext, EventResult, ViewEvent};
use viewkit::platform::PointerButton;
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

type SelectionHandler = dyn Fn(&Path) -> Result<(), String>;

#[derive(Clone, Debug)]
pub struct SavePanelOptions {
    pub title: String,
    pub suggested_name: String,
    pub initial_directory: Option<PathBuf>,
    pub root_directory: Option<PathBuf>,
    pub confirms_replacement: bool,
}

impl Default for SavePanelOptions {
    fn default() -> Self {
        Self {
            title: String::from("Save As"),
            suggested_name: String::from("Untitled"),
            initial_directory: None,
            root_directory: None,
            confirms_replacement: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct OpenPanelOptions {
    pub title: String,
    pub initial_directory: Option<PathBuf>,
    pub root_directory: Option<PathBuf>,
}

impl Default for OpenPanelOptions {
    fn default() -> Self {
        Self {
            title: String::from("Open"),
            initial_directory: None,
            root_directory: None,
        }
    }
}

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    name: String,
    directory: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Open,
    Save { confirms_replacement: bool },
}

struct PanelState {
    visible: Cell<bool>,
    mode: Mode,
    root: PathBuf,
    directory: RefCell<PathBuf>,
    entries: RefCell<Vec<Entry>>,
    selected: Cell<Option<usize>>,
    scroll: Cell<f32>,
    last_click: RefCell<Option<(usize, Instant)>>,
    name: TextFieldInteractionState,
    error: RefCell<Option<String>>,
    pending_replace: RefCell<Option<PathBuf>>,
    handler: Rc<SelectionHandler>,
}

impl PanelState {
    fn navigate(&self, path: &Path) {
        let Ok(path) = fs::canonicalize(path) else {
            *self.error.borrow_mut() =
                Some(String::from("The selected folder is no longer available."));
            return;
        };
        if !path.starts_with(&self.root) || !path.is_dir() {
            *self.error.borrow_mut() = Some(String::from(
                "This folder is outside the permitted location.",
            ));
            return;
        }
        *self.directory.borrow_mut() = path.clone();
        *self.entries.borrow_mut() = entries(&path, &self.root);
        self.selected.set(None);
        self.scroll.set(0.0);
        self.last_click.borrow_mut().take();
        self.error.borrow_mut().take();
    }

    fn cancel(&self) {
        self.visible.set(false);
        self.name.set_focused(false);
        self.error.borrow_mut().take();
        self.pending_replace.borrow_mut().take();
    }

    fn accept(&self) {
        let destination = match self.mode {
            Mode::Open => {
                let Some(index) = self.selected.get() else {
                    *self.error.borrow_mut() = Some(String::from("Select a file to open."));
                    return;
                };
                let Some(entry) = self.entries.borrow().get(index).cloned() else {
                    *self.error.borrow_mut() =
                        Some(String::from("The selected file is no longer available."));
                    return;
                };
                if entry.directory {
                    self.navigate(&entry.path);
                    return;
                }
                let Ok(path) = fs::canonicalize(&entry.path) else {
                    *self.error.borrow_mut() =
                        Some(String::from("The selected file is no longer available."));
                    return;
                };
                if !path.starts_with(&self.root) || !path.is_file() {
                    *self.error.borrow_mut() =
                        Some(String::from("This file cannot be opened from here."));
                    return;
                }
                path
            }
            Mode::Save {
                confirms_replacement,
            } => {
                let filename = self.name.value();
                let filename = filename.trim();
                if !valid_file_name(filename) {
                    *self.error.borrow_mut() = Some(String::from("Enter a valid file name."));
                    return;
                }
                let Ok(directory) = fs::canonicalize(self.directory.borrow().as_path()) else {
                    *self.error.borrow_mut() =
                        Some(String::from("The selected folder is no longer available."));
                    return;
                };
                if !directory.starts_with(&self.root) {
                    *self.error.borrow_mut() = Some(String::from(
                        "This folder is outside the permitted location.",
                    ));
                    return;
                }
                let destination = directory.join(filename);
                if destination.is_dir() {
                    *self.error.borrow_mut() =
                        Some(String::from("A folder already uses this name."));
                    return;
                }
                if confirms_replacement
                    && destination.exists()
                    && self.pending_replace.borrow().as_ref() != Some(&destination)
                {
                    *self.pending_replace.borrow_mut() = Some(destination);
                    *self.error.borrow_mut() = Some(String::from(
                        "A file already uses this name. Select Save again to replace it.",
                    ));
                    return;
                }
                destination
            }
        };

        match (self.handler)(&destination) {
            Ok(()) => self.cancel(),
            Err(error) => *self.error.borrow_mut() = Some(error),
        }
    }
}

struct FilePanel {
    title: String,
    state: Rc<PanelState>,
    name_field: TextField,
    up: Button,
    cancel: Button,
    accept: Button,
}

#[derive(Clone, Copy)]
struct Geometry {
    dialog: Rect,
    up: Rect,
    location: Rect,
    list: Rect,
    name: Option<Rect>,
    error: Rect,
    cancel: Rect,
    accept: Rect,
}

impl FilePanel {
    fn new(
        title: String,
        accept_label: &'static str,
        mode: Mode,
        suggested_name: String,
        initial_directory: Option<PathBuf>,
        root_directory: Option<PathBuf>,
        handler: impl Fn(&Path) -> Result<(), String> + 'static,
    ) -> Self {
        let root = canonical_root(root_directory);
        let initial = initial_directory
            .and_then(|path| fs::canonicalize(path).ok())
            .filter(|path| path.starts_with(&root) && path.is_dir())
            .unwrap_or_else(|| root.clone());
        let name = TextFieldInteractionState::new();
        name.set_value(suggested_name);
        let state = Rc::new(PanelState {
            visible: Cell::new(false),
            mode,
            root: root.clone(),
            directory: RefCell::new(initial.clone()),
            entries: RefCell::new(entries(&initial, &root)),
            selected: Cell::new(None),
            scroll: Cell::new(0.0),
            last_click: RefCell::new(None),
            name: name.clone(),
            error: RefCell::new(None),
            pending_replace: RefCell::new(None),
            handler: Rc::new(handler),
        });

        let up_state = Rc::clone(&state);
        let up = Button::new("Up")
            .size(ButtonSize::Small)
            .style(ButtonStyle::Ghost)
            .on_click(move || {
                let parent = up_state
                    .directory
                    .borrow()
                    .parent()
                    .map(Path::to_path_buf)
                    .filter(|path| path.starts_with(&up_state.root));
                if let Some(parent) = parent {
                    up_state.navigate(&parent);
                }
            });
        let cancel_state = Rc::clone(&state);
        let cancel = Button::new("Cancel")
            .size(ButtonSize::Small)
            .style(ButtonStyle::Standard)
            .on_click(move || cancel_state.cancel());
        let accept_state = Rc::clone(&state);
        let accept = Button::new(accept_label)
            .size(ButtonSize::Small)
            .style(ButtonStyle::Accent)
            .on_click(move || accept_state.accept());

        Self {
            title,
            name_field: TextField::with_interaction(name).placeholder("File name"),
            state,
            up,
            cancel,
            accept,
        }
    }

    fn show(&self) {
        self.state.error.borrow_mut().take();
        self.state.pending_replace.borrow_mut().take();
        self.state.name.set_focused(self.state.mode != Mode::Open);
        self.state.visible.set(true);
    }

    fn is_visible(&self) -> bool {
        self.state.visible.get()
    }

    fn geometry(&self, bounds: Rect) -> Geometry {
        let width = (bounds.size.width - 64.0).clamp(420.0, 680.0);
        let height = (bounds.size.height - 64.0).clamp(360.0, 520.0);
        let dialog = Rect::new(
            bounds.origin.x + (bounds.size.width - width) / 2.0,
            bounds.origin.y + (bounds.size.height - height) / 2.0,
            width,
            height,
        );
        let inset = 20.0;
        let top = dialog.origin.y + 54.0;
        let up = Rect::new(dialog.origin.x + inset, top, 56.0, 30.0);
        let location = Rect::new(dialog.origin.x + 88.0, top, width - 108.0, 30.0);
        let has_name = self.state.mode != Mode::Open;
        let bottom_controls_y = dialog.origin.y + height - 86.0;
        let list_bottom = if has_name {
            bottom_controls_y - 12.0
        } else {
            bottom_controls_y - 12.0
        };
        let list = Rect::new(
            dialog.origin.x + inset,
            top + 42.0,
            width - inset * 2.0,
            (list_bottom - (top + 42.0)).max(80.0),
        );
        let name = has_name.then(|| {
            Rect::new(
                dialog.origin.x + inset,
                bottom_controls_y,
                width - 236.0,
                32.0,
            )
        });
        let error_y = if has_name {
            dialog.origin.y + height - 48.0
        } else {
            bottom_controls_y + 5.0
        };
        Geometry {
            dialog,
            up,
            location,
            list,
            name,
            error: Rect::new(dialog.origin.x + inset, error_y, width - 220.0, 22.0),
            cancel: Rect::new(
                dialog.origin.x + width - 188.0,
                bottom_controls_y,
                76.0,
                32.0,
            ),
            accept: Rect::new(
                dialog.origin.x + width - 100.0,
                bottom_controls_y,
                80.0,
                32.0,
            ),
        }
    }
}

impl View for FilePanel {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(constraints.maximum)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let geometry = self.geometry(bounds);
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
            .paint(geometry.dialog, context);
        Text::styled(self.title.clone(), TextRole::TitleSmall).paint(
            Rect::new(
                geometry.dialog.origin.x + 20.0,
                geometry.dialog.origin.y + 18.0,
                geometry.dialog.size.width - 40.0,
                26.0,
            ),
            context,
        );
        self.up.paint(geometry.up, context);
        Text::body(self.state.directory.borrow().display().to_string())
            .paint(geometry.location, context);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.colors.surface_subtle))
            .radius(CornerRadius::Small)
            .border(BorderStyle::custom(context.theme.colors.border, 1.0))
            .paint(geometry.list, context);
        context.display_list.push(DrawCommand::PushClip {
            rect: geometry.list,
        });
        let row_height = 34.0;
        for (index, entry) in self.state.entries.borrow().iter().enumerate() {
            let row = Rect::new(
                geometry.list.origin.x,
                geometry.list.origin.y + index as f32 * row_height - self.state.scroll.get(),
                geometry.list.size.width,
                row_height,
            );
            if row.origin.y + row.size.height <= geometry.list.origin.y
                || row.origin.y >= geometry.list.origin.y + geometry.list.size.height
            {
                continue;
            }
            if self.state.selected.get() == Some(index) {
                Rectangle::new()
                    .color(RectangleColor::Custom(context.theme.colors.accent_soft))
                    .paint(row, context);
            }
            let label = if entry.directory {
                format!("{} /", entry.name)
            } else {
                entry.name.clone()
            };
            Text::body(label).paint(
                Rect::new(
                    row.origin.x + 12.0,
                    row.origin.y + 7.0,
                    row.size.width - 24.0,
                    20.0,
                ),
                context,
            );
        }
        context.display_list.push(DrawCommand::PopClip);
        if let Some(name) = geometry.name {
            self.name_field.paint(name, context);
        }
        if let Some(error) = self.state.error.borrow().as_ref() {
            Text::metadata(error.clone()).paint(geometry.error, context);
        }
        self.cancel.paint(geometry.cancel, context);
        self.accept.paint(geometry.accept, context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let geometry = self.geometry(bounds);
        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Escape,
                ..
            }
        ) {
            self.state.cancel();
            context.request_redraw();
            return EventResult::Consumed;
        }
        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Enter,
                ..
            }
        ) {
            self.state.accept();
            context.request_redraw();
            return EventResult::Consumed;
        }
        let mut result = EventResult::Ignored;
        if let Some(name) = geometry.name {
            let previous_name = self.state.name.value();
            result = result.merge(self.name_field.handle_event(name, event, context));
            if self.state.name.value() != previous_name {
                self.state.pending_replace.borrow_mut().take();
                self.state.error.borrow_mut().take();
            }
        }
        result = result
            .merge(self.up.handle_event(geometry.up, event, context))
            .merge(self.cancel.handle_event(geometry.cancel, event, context))
            .merge(self.accept.handle_event(geometry.accept, event, context));
        if let ViewEvent::Scroll {
            position, delta_y, ..
        } = event
            && geometry.list.contains(*position)
        {
            let maximum = (self.state.entries.borrow().len() as f32 * 34.0
                - geometry.list.size.height)
                .max(0.0);
            self.state
                .scroll
                .set((self.state.scroll.get() - *delta_y * 34.0).clamp(0.0, maximum));
            context.request_redraw_in(geometry.list);
            return EventResult::Consumed;
        }
        if let ViewEvent::PointerReleased {
            position,
            button: PointerButton::Primary,
        } = event
            && geometry.list.contains(*position)
        {
            let index =
                ((position.y - geometry.list.origin.y + self.state.scroll.get()) / 34.0) as usize;
            if let Some(entry) = self.state.entries.borrow().get(index).cloned() {
                let now = Instant::now();
                let double_click =
                    self.state
                        .last_click
                        .borrow()
                        .as_ref()
                        .is_some_and(|(previous, instant)| {
                            *previous == index
                                && now.saturating_duration_since(*instant)
                                    <= Duration::from_millis(500)
                        });
                *self.state.last_click.borrow_mut() = Some((index, now));
                self.state.selected.set(Some(index));
                self.state.error.borrow_mut().take();
                if entry.directory && double_click {
                    self.state.navigate(&entry.path);
                } else if !entry.directory {
                    if self.state.mode == Mode::Open && double_click {
                        self.state.accept();
                    } else if self.state.mode != Mode::Open {
                        self.state.name.set_value(entry.name);
                        self.state.pending_replace.borrow_mut().take();
                    }
                }
                context.request_redraw();
            }
            return EventResult::Consumed;
        }
        if result.is_consumed() {
            result
        } else {
            EventResult::Consumed
        }
    }
}

#[derive(Clone)]
pub struct SavePanel(Rc<FilePanel>);

impl SavePanel {
    pub fn new(
        options: SavePanelOptions,
        handler: impl Fn(&Path) -> Result<(), String> + 'static,
    ) -> Self {
        Self(Rc::new(FilePanel::new(
            options.title,
            "Save",
            Mode::Save {
                confirms_replacement: options.confirms_replacement,
            },
            options.suggested_name,
            options.initial_directory,
            options.root_directory,
            handler,
        )))
    }

    pub fn show(&self) {
        self.0.show();
    }

    pub fn is_visible(&self) -> bool {
        self.0.is_visible()
    }
}

impl View for SavePanel {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.0.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.0.paint(bounds, context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        self.0.handle_event(bounds, event, context)
    }
}

#[derive(Clone)]
pub struct OpenPanel(Rc<FilePanel>);

impl OpenPanel {
    pub fn new(
        options: OpenPanelOptions,
        handler: impl Fn(&Path) -> Result<(), String> + 'static,
    ) -> Self {
        Self(Rc::new(FilePanel::new(
            options.title,
            "Open",
            Mode::Open,
            String::new(),
            options.initial_directory,
            options.root_directory,
            handler,
        )))
    }

    pub fn show(&self) {
        self.0.show();
    }

    pub fn is_visible(&self) -> bool {
        self.0.is_visible()
    }
}

impl View for OpenPanel {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.0.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.0.paint(bounds, context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        self.0.handle_event(bounds, event, context)
    }
}

fn canonical_root(explicit: Option<PathBuf>) -> PathBuf {
    explicit
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .and_then(|path| fs::canonicalize(path).ok())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|path| fs::canonicalize(path).ok())
        })
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn entries(directory: &Path, root: &Path) -> Vec<Entry> {
    let mut entries = fs::read_dir(directory)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if metadata.is_dir()
                && fs::canonicalize(&path)
                    .ok()
                    .is_none_or(|canonical| !canonical.starts_with(root))
            {
                return None;
            }
            Some(Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path,
                directory: metadata.is_dir(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right.directory.cmp(&left.directory).then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
    });
    entries
}

fn valid_file_name(name: &str) -> bool {
    let path = Path::new(name);
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.bytes().any(|byte| byte == b'/' || byte == 0)
        && path.components().count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mochios-appkit-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn save_names_cannot_escape_the_selected_directory() {
        assert!(valid_file_name("notes.txt"));
        assert!(!valid_file_name("../notes.txt"));
        assert!(!valid_file_name("folder/notes.txt"));
        assert!(!valid_file_name(".."));
        assert!(!valid_file_name(""));
    }

    #[test]
    fn save_panel_requires_a_second_accept_before_replacement() {
        let root = temporary_directory("save-panel");
        let destination = root.join("notes.txt");
        fs::write(&destination, "old").unwrap();
        let calls = Rc::new(Cell::new(0));
        let handler_calls = Rc::clone(&calls);
        let panel = SavePanel::new(
            SavePanelOptions {
                suggested_name: String::from("notes.txt"),
                initial_directory: Some(root.clone()),
                root_directory: Some(root.clone()),
                ..SavePanelOptions::default()
            },
            move |_| {
                handler_calls.set(handler_calls.get() + 1);
                Ok(())
            },
        );
        panel.show();
        panel.0.state.accept();
        assert_eq!(calls.get(), 0);
        assert_eq!(
            panel.0.state.pending_replace.borrow().as_ref(),
            Some(&destination)
        );
        panel.0.state.accept();
        assert_eq!(calls.get(), 1);
        assert!(!panel.is_visible());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cloned_file_panels_share_presentation_state() {
        let root = temporary_directory("panel-presentation");
        let save_panel = SavePanel::new(
            SavePanelOptions {
                initial_directory: Some(root.clone()),
                root_directory: Some(root.clone()),
                ..SavePanelOptions::default()
            },
            |_| Ok(()),
        );
        let save_action_panel = save_panel.clone();
        save_action_panel.show();
        assert!(save_panel.is_visible());
        save_panel.0.state.cancel();
        assert!(!save_action_panel.is_visible());

        let open_panel = OpenPanel::new(
            OpenPanelOptions {
                initial_directory: Some(root.clone()),
                root_directory: Some(root.clone()),
                ..OpenPanelOptions::default()
            },
            |_| Ok(()),
        );
        let open_action_panel = open_panel.clone();
        open_action_panel.show();
        assert!(open_panel.is_visible());
        open_panel.0.state.cancel();
        assert!(!open_action_panel.is_visible());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn open_panel_rejects_a_selection_outside_its_root() {
        let root = temporary_directory("open-root");
        let outside = temporary_directory("open-outside");
        let outside_file = outside.join("outside.txt");
        fs::write(&outside_file, "outside").unwrap();
        let called = Rc::new(Cell::new(false));
        let handler_called = Rc::clone(&called);
        let panel = OpenPanel::new(
            OpenPanelOptions {
                initial_directory: Some(root.clone()),
                root_directory: Some(root.clone()),
                ..OpenPanelOptions::default()
            },
            move |_| {
                handler_called.set(true);
                Ok(())
            },
        );
        panel.0.state.entries.borrow_mut().push(Entry {
            path: outside_file,
            name: String::from("outside.txt"),
            directory: false,
        });
        panel.0.state.selected.set(Some(0));
        panel.0.state.accept();
        assert!(!called.get());
        assert!(panel.0.state.error.borrow().is_some());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
