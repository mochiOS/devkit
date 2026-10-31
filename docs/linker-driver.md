# mochiOS ld

`crates/linker-driver`は、SDKのリンク設定を加えて`ld.lld`と既存の`msign elf`を
実行するドライバです。ELF配置・relocation、暗号処理、鍵管理は実装しません。

```sh
cargo build --manifest-path tools/devkit/Cargo.toml -p mochios-linker-driver
MOCHIOS_SDK=/path/to/sdk ld main.o -o app
```

後者はビルドした`ld`、`ld.lld`、`msign`を配置した環境での操作です。
SDK生成やツールのインストールはこのドライバでは行いません。

## SDKとツールの選択

SDKは`--mochios-sdk`、`MOCHIOS_SDK`、明示した`--sysroot=SDK/sysroot`の親、
ドライバの`bin/`の親の順で選びます。必須ファイルの欠落・空ファイルはエラーです。
SDKの`lib/crt0.o`、`lib/linker.ld`、`lib/libmochi_user_newlib_runtime.a`、
`sysroot/lib/libc.a`、`sysroot/lib/libm.a`とcompiler runtimeを使用します。
補助オブジェクトは存在だけでは自動追加しません。必要なら通常の入力として明示します。

compiler runtimeは`--compiler-runtime`またはSDKの`manifest.toml`の
`compiler_runtime`で選びます。manifestのない従来SDKは`libgcc`です。
`libgcc`は`lib/libgcc.a`、`compiler-rt`は`lib/libclang_rt.builtins-x86_64.a`です。
不正なmanifest、未知のruntime、x86_64以外のarchitectureは拒否します。
SDKとsysrootのlibrary search pathも加え、ユーザー指定の`-L`・`-l`は保持します。

backendは`--mochios-lld`、`MOCHIOS_LLD`、隣接した`ld.lld`、PATHの`ld.lld`の順です。
別のリンカへのfallbackはありません。msignも`--mochios-msign`、`MOCHIOS_MSIGN`、
隣接した`msign`、PATHの順です。自身の実行体を再帰起動する指定は拒否します。

## リンクと署名

既定はx86-64のstatic ET_EXECで、`-static -no-pie -z noexecstack`を加えます。
`-T` / `--script`を指定した場合はSDKのscriptを加えません。
`-nostartfiles`はcrt0、`-nodefaultlibs`はruntimeと標準ライブラリを省略します。
`-nostdlib`は両方を省略します。これらはdriverが消費し、GCC専用引数としてLLDへ渡しません。
`-r`はSDK設定を追加せず、ET_RELを生成して署名しません。

通常の最終出力では、次の順で既存msignを呼びます。

```text
ld.lld ... -o PRIVATE/linked.elf
msign elf sign PRIVATE/linked.elf --adhoc
msign elf verify PRIVATE/linked.elf
rename PRIVATE/linked.elf OUTPUT
```

PRIVATEは出力ディレクトリ内に原子的に作る0700の一時ディレクトリです。
link・sign・verifyの失敗では既存OUTPUTを置き換えません。
成功後はファイルをsyncし、renameして親ディレクトリもsyncします。
rename後のdirectory syncが失敗した場合、公開済みでもエラーを返します。
通常の失敗経路では一時ディレクトリを解放しますが、プロセス強制終了後の回収は別途必要です。

`--sign-key PRIVATE --public-key PUBLIC`ではmsignへ明示的に鍵を渡して署名・検証します。
その失敗からAdHocへfallbackしません。鍵またはそのhard linkを出力先にする指定も拒否します。
`--no-sign`は明示的に署名を省きます。AdHoc署名は信頼やCapability許可を意味しません。

子ツールの終了コードを返し、signal終了は128+signalです。起動・SDK・公開の失敗は1です。
`-v`は実行コマンド、`--dry-run`は実行・出力作成なしで予定コマンドを表示します。
`--version`はSDKを要求せずLLDのversionを表示します。

## 対応範囲と残件

初期版はstatic ELFと`-r`に対応します。shared / PIEとresponse fileは明示的に拒否します。
response fileをそのまま転送すると非公開の`-o`を上書きできるため、展開・正規化を実装するまで
転送しません。`-o FILE`、`-oFILE`、`--output[=]FILE`は一つの非公開出力へ正規化します。
パスはシェル展開せずargvとして渡し、非UTF-8のUnixパスも保持します。

テストは`services/posix/tests/linker_driver.rs`にあります。失敗注入11件に加え、
実LLD・msignでの署名と改変拒否、実driver CLIと既存SDK入力によるCRT・runtime・libcの
リンクと失敗時の出力維持を、明示実行する2件で確認します。
mochiOS向けdriverのビルドも確認していますが、ゲストでのdriver / msignの実行は未検証です。
既存msign全体のnative buildは、証明書取得用ネットワーク依存のmio / socket2で失敗しました。
mioのmochiOS向けselectorとwaker、およびsocket2が参照するlibc宣言が不足しています。
ELF署名の実装を複製せず、ローカル署名のビルド範囲とネットワーク依存の移植を整理する必要があります。

通常clangのdefault driver設定、ゲストへのツール配置、ELF内manifest宣言、実行時の
署名検証と共通Capability同意の接続はまだありません。この段階で通常の
`clang main.c -o main; ./main`や完全なPOSIX環境を完成扱いにはしません。
