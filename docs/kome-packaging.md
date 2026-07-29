# Kome Packaging Guide

新規projectは任意の有効なreverse-domain Package IDで作成できます。

```sh
kome new Example --id com.example.application --vendor "Example Developer"
cd Example
kome build
kome pack
```

`Kome.toml`の主要部分:

```toml
[package]
name = "Example"
id = "com.example.application"
version = "0.1.0"
vendor = "Example Developer"

[developer]
id = "019f9e5ac6687902b0e72fe53abfbef1"
```

`[developer]`は任意です。省略時はユーザー設定、またはAccountの発行可能Developerから
`kome sign`が選択します。

`kome pack`は次を生成します。

```text
dist/Example-unsigned.mpkg
target/mpkg-staging/manifest.toml
target/mpkg-staging/payload/bundle/entry.elf
```

生成manifestには実payloadから計算したsizeと`sha256:` digestが入ります。
`target/mpkg-staging`を作り直してから`mpack create`へ渡すため、同じ入力から同じMPKGを
生成します。legacy `.pkg`は生成しません。

通常はbuildとpackも自動実行する次のコマンドだけで十分です。

```sh
kome sign
```
