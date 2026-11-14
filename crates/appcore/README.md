# AppCore

AppCore is the application framework for mochiOS. It brings together ViewKit's
user interface components and APIs for integrating with the mochiOS desktop.
Applications use AppCore to build their UI and work with OS services through a
common application API.

It provides:

- Windows, layout, controls and application menus through ViewKit.
- Document windows, file panels, undo, recovery and session restoration.
- Desktop integration, including the clipboard, notifications and default applications.
- Private application storage and background tasks that deliver results to the UI.

Rust is the supported application API. C headers and experimental Kome wrappers
are also included.

## Getting started

Add the crate under the name `appcore`:

```toml
[dependencies]
appcore = { package = "mochios-appcore", git = "https://github.com/mochiOS/devkit" }
```

Implement `App` and start it with `appcore::run`:

```rust,no_run
use appcore::prelude::*;

struct ExampleApp;

impl App for ExampleApp {
    type Body = Text;

    fn new() -> Self {
        Self
    }

    fn window(&self) -> WindowOptions {
        WindowOptions::new("Example")
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        Text::new("Hello from mochiOS")
    }
}

fn main() -> Result<(), ViewKitError> {
    appcore::run::<ExampleApp>()
}
```

## Documentation

- [Usage guide](docs/usage.md): documents, storage, background tasks and language bindings.
- [API modules](src/lib.rs): public types and their documentation.
- [mochiOS developer kit](../../readme.md): building and packaging applications.
