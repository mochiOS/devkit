# Kome Packaging Guide

Komeの標準package flowはMPKG v1です。

```sh
kome new Example --id org.example.application --developer org.example.developer
cd Example
kome build
kome pack
```

`kome pack`は`Kome.toml`と`target/debug/entry.elf`から次を生成します。

```text
dist/Example-unsigned.mpkg
target/mpkg-staging/manifest.toml
target/mpkg-staging/payload/bundle/entry.elf
```

`target/mpkg-staging/manifest.toml`には実payloadから計算した`size`と
`sha256:` digestが入ります。`kome pack`はこのstaging directoryを作り直してから
`mpack create`へ渡します。

legacy `.pkg`が必要な場合だけ明示します。

```sh
kome pack --legacy
```

legacy `.pkg`はAppStore向け標準形式ではありません。
