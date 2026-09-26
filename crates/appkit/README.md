# AppKit

This crate is the supported application-facing entry point for mochiOS. It
combines ViewKit with capability-checked desktop integration instead of making
each application depend on service protocols directly.

## Rust

```rust,ignore
use appkit::prelude::*;

struct ExampleApp;

impl App for ExampleApp {
    type Body = Text;

    fn window(&self) -> WindowOptions {
        WindowOptions::new("Example")
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        Text::new("Hello from mochiOS")
    }
}

fn main() -> Result<(), ViewKitError> {
    appkit::run::<ExampleApp>()
}
```

Desktop integration is grouped by purpose:

- `clipboard`: typed shared clipboard access
- `content_type`: validated content identifiers, conformance and extension inference
- `document`: opening documents and managing default applications
- `DocumentController`: Open, Save, Save As, Revert, autosave, edited-state
  tracking and unsaved-changes confirmation for file-backed document windows
- `OpenPanel` and `SavePanel`: workspace-owned panels on mochiOS. Files.app
  performs browsing in a separate trusted process; the calling application
  receives access only to the selected file for the lifetime of its process
- `ApplicationMenuItem::command` and `CommandTarget`: menu commands routed
  through the focused responder branch, with automatic enabled/checked/title
  validation from the first responder
- `UndoManager` and `UndoResponder`: grouped application-level undo and redo
- `RecoveryStore`: atomic snapshots for unsaved document contents
- `SessionStore`: atomic restoration of independent document windows, paths,
  recovery identifiers, and logical window frames
- `viewkit` / `prelude`: windows, layout, controls, events, appearance, per-window
  accessibility snapshots and the platform bridge boundary
- `FileDropTarget`: native file Drag & Drop with filtering and observable hover state

Applications can create independent windows with `request_new_window()` and
provide their configuration and content through `App::window_for` and
`App::body_for`. A document controller in a multi-window application should use
`on_close` with `request_close_window(window_id)` so a completed save closes
only its owning window.

Most APIs do not grant authority. The application manifest must request the
corresponding capabilities, such as `clipboard.read`, `clipboard.write`,
`file-association.read`, and `file-association.write`. File panels are the
exception: an app declaring `fs.read.user` or `fs.write.user` receives a scoped
grant for the path that the user selected, never broad access to the home
directory.

## C, Clang and Kome

The crate also builds `staticlib` and `cdylib` outputs. The public headers are
in `include/`:

```c
#include <mochios.h>

int32_t status = mochios_clipboard_set_text(mochios_c_string("hello"));
```

`mochios.h` is the umbrella header and includes ViewKit's C API. `mochios_abi.h`
contains only the stable system-integration
ABI and fixed-width C types, which is suitable for generated Kome bindings.

ABI rules:

- version encoding is `0x00MMmmpp` and breaking layouts require a new major;
- all strings are UTF-8 pointer/length views and do not require NUL termination;
- no Rust-owned allocation crosses the ABI boundary;
- variable output uses caller-owned buffers and reports the required length;
- functions catch Rust panics and return `MOCHIOS_STATUS_PANIC`;
- `MOCHIOS_STATUS_SYSTEM_ERROR` details are available from
  `mochios_last_system_error()` on the calling thread;
- invalid UTF-8, invalid role bits, null pointers and insufficient buffers fail
  closed before an OS service is invoked.

The ViewKit ABI has its own independent version. Consumers must check both
`mochios_abi_version()` and `vk_abi_version()` when loading shared libraries.
