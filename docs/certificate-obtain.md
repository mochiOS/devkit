# Low-level Certificate Guide

通常の開発者はCertificateを手動取得しません。`kome sign`が認証済みAccountのDeveloperを
解決し、必要な場合だけDeveloperCAへ発行を要求します。

DeveloperCAへ送る情報:

```text
Developer ID
application.pub
Package ID
全binary.requiresの和集合
```

送らない情報:

```text
application.key
CLI refresh token
MPKG本体とpayload
source code
build成果物
ローカルpath
```

取得したMCER v1は`keys/developer.cert`へ保存する前に、canonical encoding、Issuer署名、
Subject公開鍵とKey ID、Developer ID、Package ID scope、Capability、有効期間を検証します。
Issuer公開鍵は認証済みHTTPS応答から取得し、`keys/developer.issuer.pub`へ保存して以後の
ローカル検証に使います。

次の低レベルコマンドは運営、fixture、API調査用です。一般向け署名フローでは要求しません。

```sh
msign certificate obtain \
  --developer 019f9e5ac6687902b0e72fe53abfbef1 \
  --public-key keys/application.pub \
  --package dist/Example-unsigned.mpkg \
  --output keys/developer.cert
```

offline fixture用の発行:

```sh
msign certificate issue \
  --issuer-key fixture-root.key \
  --subject-public-key keys/application.pub \
  --developer-id 019f9e5ac6687902b0e72fe53abfbef1 \
  --serial 1 \
  --not-before 1700000000 \
  --not-after 1800000000 \
  --scope exact:com.example.application \
  --capability window.create \
  --output keys/developer.cert
```

`msign certificate issue`へapplication秘密鍵を渡す必要はありません。
