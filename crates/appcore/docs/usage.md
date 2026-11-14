# AppCore usage guide

This guide covers application API usage and SDK maintenance. See the
[AppCore overview](../README.md) for the framework's role and a minimal application.

## Files and application storage

`OpenPanel::with_file` and `SavePanel::with_file` deliver an owned `DocumentFile`
from the system picker without requiring `fs.read.user` or `fs.write.user`.
Open panels provide read access by default; use `allows_editing(true)` when the
application needs to save changes to the selected file. The returned resource
carries access authority; its path is display metadata and should not be reopened.

Legacy path callbacks and `DocumentFile::open/create` use the caller's existing
filesystem authority. A path alone grants no access to the selected file or its
neighbours. The handle-based picker API described here is available through Rust;
the existing C and Kome path callbacks retain their current permission requirements.

`ApplicationData::open()` provides private storage for the current application
and OS account. Pass file names to `read`, `write` and `remove`; applications do
not select a UID, publisher or storage directory. These operations require no
filesystem capability.

Desktop APIs follow OS authorization. Declare the capabilities required by each
API in the application manifest. Default-application changes are confirmed by
the OS; an ordinary application can propose itself for its declared document types.

## Document windows

Applications can create independent windows with `request_new_window()` and
provide their configuration and content through `App::window_for` and
`App::body_for`. A document controller in a multi-window application should use
`on_close` with `request_close_window(window_id)` so a completed save closes
only its owning window.

ファイルを扱うRustアプリは`DocumentController::new_background`と`DocumentIo`で
文書形式を登録できます。使用例はEdit.appの`TextFormat`と
`make_document_controller`を参照してください。

- `DocumentFormat::read/write`はワーカー上で動きます。UIのStateは持たせません。
- `DocumentIo::new`のsnapshot/applyはUI上で動き、内容の所有権だけをワーカーと移動します。
- Open／Save Asは既存のFiles.appパネルを使います。パネルの終了はI/O成功の通知ではありません。
- `request_save`は`SaveRequest`で「同期完了・非同期受付・保存先選択・実行中・受付失敗」を区別します。
  非同期の完了待ちはせず、`is_busy`と`status`で表示を更新します。
- 保存済み世代は保存開始時のスナップショットに結び付きます。保存中の追加編集を消さず、
  未保存の追加編集がある場合は保留中のOpen／Closeを続行しません。
- 読み込み中に編集された場合も、読み込み結果でその編集を上書きしません。
- `open_file(DocumentFile::from_received(document))`はIPCで受け取った資源を使い、
  表示用パスで開き直しません。現行の受信資源は読み取り専用とし、保存はSave Asを使います。
- I/O中はウィンドウ終了・Quitを保留します。失敗時は文書を保存済みにせず、標準エラーを表示します。
- 既存の`DocumentController::new`は同期の互換APIです。`save`のboolを非同期の成否として
  解釈せず、新しいコードでは`request_save`を使ってください。

`DocumentFile::replace_contents`の保存方式は、取得した資源によって異なります。
システムパネルから取得した保存先は、OSが保持する単一の宛先に対し、同期済みの
一時ファイルで原子的に置換します。`DocumentFile::open/create`でパスから開いた
通常のファイルには直接書き込み、長さ調整と`sync_all`を行います。後者は途中の失敗で
部分的な書き込みが残る場合があります。書き込み開始後の強制キャンセルや、
自動ロールバックは提供しません。

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
        std::fs::read_to_string("./note.txt").map_err(TaskError::Operation)
    },
    |result| { /* UIのStateを更新する */ },
)?;
# Ok::<(), appcore::task::TaskStartError>(())
```

- `spawn`の戻り値は受付結果、完了コールバックの引数は処理結果です。
- `TaskHandle::cancel`は協調的な中止要求です。終了やロールバックを保証しません。
- `TaskScope`を破棄すると中止を要求し、その後のUI通知を解除します。終了待ちはしません。
- 保存・インストールなどを完了させる必要がある場合は、アプリの終了確認で保護します。
- `App::close_requested`はウィンドウを閉じる操作とアプリ全体のQuitの両方で呼びます。
  falseを返したQuitは取り消し、処理完了後に勝手に終了しません。
- `spawn_with_progress`は途中経過を最新値に集約し、完了結果は別に保持します。
- `TaskSlot`は検索などの最後の要求だけを表示する用途です。書き込みには使いません。
- `TaskExecutor::new`で実行数・待ち行列を制限できます。Scopeの未配送結果も64件に制限します。
- 権限は追加しません。失敗時も既存のCapability制約に従います。

Komeの`BackgroundTasks`は、ネイティブワーカーによる上限付きUTF-8ファイル読み取りを
提供します。Komeの`task / wait`は同一スレッドの協調実行であり、用途が異なります。
Komeクロージャをネイティブワーカーへ渡すAPIは提供していません。
完了クロージャはUIでのみ保持・実行・解放します。
APIの引数・寿命・失敗条件・使用例はRust/Komeの公開宣言のDocを参照してください。

DevKitリポジトリのルートから、KomeのバックグラウンドAPIの構文・型を
実コンパイラーで検査できます。

```sh
python3 crates/appcore/scripts/check-background.py --compiler /path/to/komec
```

この検査は新APIとその依存定義に限定したコンパイル検査です。
`--viewkit /path/to/ViewKit/lib`を追加すると、AppCoreとViewKitの全Komeラッパーも
読み込んで互換性を検査します。
ネイティブABIの配送・所有権は`ffi_task::tests`、タスクの状態遷移は`task::tests`で検査します。

## C, Clang and Kome

The crate also builds `staticlib` and `cdylib` outputs. The public headers are
in [include/](../include/):

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
- catchable Rust panics return `MOCHIOS_STATUS_PANIC`; a `panic=abort` build
  terminates the process;
- `MOCHIOS_STATUS_SYSTEM_ERROR` details are available from
  `mochios_last_system_error()` on the calling thread;
- invalid UTF-8, invalid role bits, null pointers and insufficient buffers fail
  closed before an OS service is invoked.

The ViewKit ABI has its own independent version. Consumers must check both
`mochios_abi_version()` and `vk_abi_version()` when loading shared libraries.

The experimental Kome wrappers load AppCore as a system package:

```toml
[dependencies]
appcore = { system = true }
```

```kome
fn main() {
    let status = Clipboard.text("Copied from Kome")
}
```

## Building SDK releases

Run `crates/appcore/scripts/release.pl` from the DevKit repository. It builds
AppCore in release mode and writes `{arch}-appcore-{version}.tar.zst` together
with `SHA256SUMS` to `target/release`. The archive can be extracted directly to
`~/.kome/appcore` and contains the Kome package, native libraries, and C headers.

Release packaging requires Python 3. Source paths in AppCore and its Rust
dependencies are remapped to stable virtual prefixes. Existing Cargo flags and
compiler wrappers are preserved. Release compilations use a policy-specific
cache under `target/appcore-release`; archives still go to `target/release`.
Before archiving, the complete staged payload is scanned for build-environment
paths. A remaining path aborts packaging; it is never silently removed from a
finished binary. This command only creates local artifacts and never publishes
a GitHub release.
