# Package Signing Guide

MPKG署名:

```sh
kome sign \
  dist/Example-unsigned.mpkg \
  --certificate keys/developer.cert \
  --key keys/application.key \
  --output dist/Example.mpkg \
  --unix-time 1750000000
```

低レベルCLI:

```sh
msign package sign \
  dist/Example-unsigned.mpkg \
  --certificate keys/developer.cert \
  --key keys/application.key \
  --output dist/Example.mpkg
```

署名前に確認するもの:

```text
inputがMPKG v1
manifest.tomlが一意
developer.certがMCER v1
application.key由来の公開鍵とcertificate Subject公開鍵の一致
package.idがcertificate scope内
全binary.requiresがcertificate allowed capabilities内
certificateが有効期間内
既署名MPKGではないこと
```

署名対象:

```text
"mochios-mpkg-manifest-v1\0" || SHA-256(manifest.toml bytes)
```

署名後に追加されるentry:

```text
signatures/developer.cert
signatures/manifest.sig
```

通常は既署名MPKGへの再署名を拒否します。明示的に置換する場合だけ
`msign package sign --replace-signature`を使用します。

ローカル検証:

```sh
kome verify dist/Example.mpkg \
  --issuer-public-key root.pub \
  --unix-time 1750000000
```
