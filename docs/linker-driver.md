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

`--manifest manifest.toml`は、同じ入力をmsignへ渡し、宣言をELF内へ埋め込んでから署名します。
clangからは`-Wl,--manifest,manifest.toml`で指定できます。後からsidecarを変更・削除しても、
ELF内の署名対象宣言は変わりません。`--no-sign` / `-r`との併用は拒否し、LLDへ転送しません。
出力でmanifest本体やそのhard linkを置換する指定も拒否します。
不正なTOMLや署名の失敗は既存ELFを維持したまま失敗します。暗黙のmanifest探索は行いません。
形式は[埋め込みmanifest](elf-signature-v1.md#embedded-manifest-declarations)を参照してください。
要求Capabilityの認可や実行時gateの強制まで完成したものではありません。

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
mochiOSの通常Terminalから専用fixtureを実行し、実clangのコンパイル、driver経由の
LLDリンク、既存msignのAdHoc署名・検証、生成ELFのstdoutと終了コード37を確認しました。
ネイティブ乱数による鍵生成と鍵付き署名・検証、一時ディレクトリ解放も成功しています。
改変拒否はmsign verifyでの確認であり、実行認可gateへの署名検証接続はまだありません。
fixtureは`services/posix/tests/native_signer_flow.rs`、記録は
`out/checks/mochios-signer-native-314-flow-second-final.png`です。

msignは`--no-default-features`でオンライン証明書取得だけを省いた同じ実装です。
通常のネットワーク依存を含むnative buildには、mioのselector / wakerとsocket2の移植が
引き続き必要です。[ビルド構成と共通公開処理](elf-signature-v1.md#native-build-profile)を参照してください。

mochiOS専用clang ToolChainのdefault設定も追加し、診断SDKの実clangへ追加target / SDK引数なしで
`clang main.c -o main`を渡す経路を確認しました。stdio / stdintを含むCを共通ldでリンク・自動署名し、
msign verifyのAdHoc判定、`./main`のstdoutと終了コード37が一致しています。
clangのpipe出力を失っていたnewlibの終了callback登録上限はソースで修正し、検証用clangへリンクしました。
供給SDKは変更・再生成しておらず、一般アプリへの同修正の反映はSDK管理側の取り込みが必要です。
記録は`out/checks/mochios-exit317-clang-positive.png`、詳細は
`docs/services/posix-native-clang-pipeline.md`です。

317時点ではfixtureの後半が約900秒で終了していました。319で静的ELFのdigestと
owned snapshotにも共有転送を接続し、compile / link失敗時の旧ELF維持、改変拒否、
cleanupまで全gateが成功しました。321では通常PATHのclang / ld / ld.lld / msignを
使って同じ全gateが成功しています。追加のSDK / target引数や診断tool directoryは使いません。
ツールと提供SDKは/system/developerへまとめ、/system/binの相対symlinkから利用します。
USER_INSTALL_DEVELOPER_TOOLS=yで既存のbuild済み入力を配置し、SDK生成は行いません。
詳細はrootのdocs/services/posix-native-development-tools.mdです。

埋め込み宣言の共通認可policyへの接続、実行時の共通署名検証とCapability同意、ネットワーク依存を含むmsign、
完全なPOSIX環境は引き続き未完了です。msign verifyの改変拒否は実行認可gateの完成とは区別します。
