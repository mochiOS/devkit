# AppCore

This crate is the supported application-facing entry point for mochiOS. It
combines ViewKit with capability-checked desktop integration instead of making
each application depend on service protocols directly.

## Rust

```rust,ignore
use appcore::prelude::*;

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
    appcore::run::<ExampleApp>()
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
- `ControlCenterItem` and `ControlCenterCard`: publish compact, declarative card
  content that Binder renders inside Control Center without loading application
  code into the desktop shell
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
`file-association.read`, `file-association.write`, and
`control-center.register`. File panels are the
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

Kome projects load AppCore as a system package and use its thin wrappers:

```toml
[dependencies]
appcore = { system = true }
```

```kome
fn main() {
    let status = Clipboard.text("Copied from Kome")
}
```

## Background tasks

`appcore::task`は、同期I/Oや計算をUIの外で実行する共通APIです。
ワーカーへ渡す入力・出力は`Send`、完了・進捗コールバックは作成元のUIスレッド専用です。
ViewKitのイベントループが通知を受けて起床するため、アプリ側の定期ポーリングは不要です。

```rust,no_run
use appcore::task::{TaskScope, TaskError, TaskResult};

let tasks = TaskScope::new()?; // ウィンドウなどの所有者に保持する
let handle = tasks.spawn(
    |task| -> TaskResult<String, std::io::Error> {
        task.check_cancelled()?;
        std::fs::read_to_string("/home/user/note.txt").map_err(TaskError::Operation)
    },
    |result| { /* UIのStateを更新する */ },
)?;
# Ok::<(), appcore::task::TaskStartError>(())
```

- `spawn`の戻り値は受付結果、完了コールバックの引数は処理結果です。
- `TaskHandle::cancel`は協調的な中止要求です。終了やロールバックを保証しません。
- `TaskScope`を破棄すると中止を要求し、その後のUI通知を解除します。終了待ちはしません。
- 保存・インストールなどを完了させる必要がある場合は、アプリの終了確認で保護します。
- `spawn_with_progress`は途中経過を最新値に集約し、完了結果は別に保持します。
- `TaskSlot`は検索などの最後の要求だけを表示する用途です。書き込みには使いません。
- `TaskExecutor::new`で実行数・待ち行列を制限できます。Scopeの未配送結果も64件に制限します。
- 権限は追加しません。失敗時も既存のCapability制約に従います。

Komeの`BackgroundTasks`は、ネイティブワーカーによる上限付きUTF-8ファイル読み取りを
提供します。Komeの`task / wait`は同一スレッドの協調実行であり、用途が異なります。
Komeクロージャをネイティブワーカーへ渡すAPIは提供していません。
完了クロージャはUIでのみ保持・実行・解放します。
APIの引数・寿命・失敗条件・使用例はRust/Komeの公開宣言のDocを参照してください。

Komeの新APIの構文・型を、実コンパイラーで検査できます。

```sh
python3 crates/appcore/scripts/check-background.py --compiler /path/to/komec
```

この検査は新APIとその依存定義に限定したコンパイル検査です。
`--viewkit /path/to/ViewKit/lib`を追加すると、AppCoreとViewKitの全Komeラッパーも
読み込んで互換性を検査します。
ネイティブABIの配送・所有権は`ffi_task::tests`、タスクの状態遷移は`task::tests`で検査します。

## Release

Run `crates/appcore/scripts/release.pl` from the DevKit repository. It builds
AppCore in release mode and writes `{arch}-appcore-{version}.tar.zst` together
with `SHA256SUMS` to `target/release`. The archive can be extracted directly to
`~/.kome/appcore` and contains the Kome package, native libraries, and C headers.
