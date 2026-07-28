# Developer Key Management

Developer application鍵はEd25519です。

```sh
kome key generate
```

既定の生成物:

```text
keys/application.key
keys/application.pub
```

低レベルCLI:

```sh
msign key generate \
  --private-key application.key \
  --public-key application.pub
```

ファイル形式:

```text
application.key  raw 32 byte Ed25519 signing keyのBase64
application.pub  raw 32 byte Ed25519 verifying keyのBase64
```

秘密鍵はstdoutへ出力しません。既存ファイルがある場合は上書きしません。
Unixでは秘密鍵を可能な範囲でowner-only permissionで作成します。

Cloudへ送るのは`application.pub`だけです。`application.key`はCloud、MPKG、
GitHub Release assetへ含めません。
