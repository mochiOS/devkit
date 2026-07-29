# Package Signing Guide

初回:

```sh
kome login
kome keygen
kome sign
```

2回目以降:

```sh
kome sign
```

`kome sign`は次を順に実行します。

1. `Kome.toml`とPackage IDを検証する
2. 入力が新しい場合にbuildとunsigned MPKG生成を行う
3. application鍵を生成または検証する
4. 保存済みCLI sessionをrefreshする
5. DeveloperCAのDeveloper一覧からDeveloperを解決する
6. 既存Developer Certificateを検証し、必要なら取得する
7. 一時MPKGへ`developer.cert`と`manifest.sig`を追加する
8. 一時MPKGをローカル検証する
9. 成功した場合だけ`dist/<name>.mpkg`へ配置する

未ログイン時はログイン方法を表示して終了し、勝手にDevice Authorizationを開始しません。
明示的に同じコマンドからログインする場合だけ`kome sign --login`を使用できます。

Developerの解決順:

1. `Kome.toml`の`[developer].id`
2. `kome developer use`で保存したdefault Developer
3. 発行可能なDeveloperが1件なら自動選択
4. 複数なら対話選択
5. 0件ならConsoleでの作成を案内

Certificate再利用時はMCER、Issuer署名、application公開鍵、Developer ID、Package ID
scope、全required Capability、有効期間を検証します。鍵、Developer、Package ID、
Capability、期限のいずれかが変わると再取得します。

署名対象:

```text
"mochios-mpkg-manifest-v1\0" || SHA-256(manifest.tomlの正確なbyte列)
```

署名後のentry:

```text
signatures/developer.cert
signatures/manifest.sig
```

ローカル検証ではMPKG header、ustar制約、MCER、Issuer署名、Certificate期限とscope、
Capability、manifest署名、payload size/digest、重複・未列挙payloadを確認します。

低レベル操作はfixtureや形式調査に限定してください。

```sh
msign package sign \
  dist/Example-unsigned.mpkg \
  --certificate keys/developer.cert \
  --key keys/application.key \
  --output dist/Example.mpkg

msign package verify \
  dist/Example.mpkg \
  --root-public-key keys/developer.issuer.pub
```
