# mochiOS developer kit

mochiOSアプリをMPKG v1としてbuild、署名、検証する開発者向けCLIです。

## Quick Start

初回だけAccountログインとEd25519 application鍵の生成を行います。

```sh
kome login
kome keygen
kome sign
```

2回目以降は、project directoryで次を実行します。

```sh
kome sign
```

`kome sign`は必要なbuildとunsigned MPKG生成を行い、認証済みAccountからDeveloperを
解決し、Developer Certificateを取得または再利用します。署名済みMPKGを一時ファイルで
検証し、成功した場合だけ`dist/<name>.mpkg`へ配置します。未ログイン時に勝手にブラウザを
開くことはありません。

新規project:

```sh
kome new Example --id com.example.application --vendor "Example Developer"
cd Example
kome login
kome keygen
kome sign
```

## Tools

- `kome`: project作成、Account session、build、pack、署名の通常入口
- `mpack`: unsigned MPKG v1の低レベル生成器
- `msign`: Ed25519鍵、MCER、MPKG署名・検証の低レベルツール
- `komeup`: Kome toolchain installer

## Generated Files

```text
Kome.toml
keys/application.key
keys/application.pub
keys/developer.cert
keys/developer.issuer.pub
target/debug/entry.elf
dist/Example-unsigned.mpkg
dist/Example.mpkg
```

`application.key`はraw 32-byte Ed25519 signing keyのBase64です。stdout、Cloud、MPKGへ
出力されず、`.gitignore`へ`keys/application.key`が重複なく追加されます。
`application.pub`だけがCertificate発行要求へ送られます。

`kome pack`はMPKG v1の32-byte headerと決定的な無圧縮ustar streamを生成します。
`manifest.toml`には実payloadのsizeとSHA-256が入り、署名前の出力には
`signatures/`がなくても構いません。

## Account And Developer

```sh
kome account
kome developer list
kome developer use 019f9e5ac6687902b0e72fe53abfbef1
kome logout
```

Developer IDは32文字の小文字16進識別子です。Developer ID自体は公開識別子であり、
credentialではありません。Package IDは`org.mochios.*`に限定されず、
`com.example.paint`や`io.github.username.tool`を使用できます。

CLI refresh credentialとsession IDはOS credential storeを優先して保存します。OS storeが
利用できない場合だけ、project外の所有者限定設定ファイルへfallbackします。Web Cookieと
access tokenは永続保存しません。

## Guides

- [Kome login guide](docs/kome-login.md)
- [Kome session guide](docs/kome-session.md)
- [Developer key management](docs/developer-key-management.md)
- [Kome package guide](docs/kome-packaging.md)
- [Package signing guide](docs/package-signing.md)
- [Package ID rules](docs/package-id.md)
- [MPKG v1 guide](docs/mpkg-v1.md)
- [AppStore publish guide](docs/appstore-publish.md)
- [Low-level Certificate guide](docs/certificate-obtain.md)
- [legacy .pkg migration guide](docs/legacy-pkg-migration.md)

## Low-level Commands

通常の開発では必要ありません。fixture、運営、形式検証用です。

```sh
mpack create --manifest manifest.toml --payload payload --output app.mpkg
msign key generate --private-key application.key --public-key application.pub
msign certificate issue \
  --issuer-key issuer.key \
  --subject-public-key application.pub \
  --developer-id 019f9e5ac6687902b0e72fe53abfbef1 \
  --serial 1 \
  --not-before 1700000000 \
  --not-after 1800000000 \
  --scope exact:com.example.application \
  --capability window.create \
  --output developer.cert
msign package sign app.mpkg --certificate developer.cert --key application.key
msign package verify app.mpkg --root-public-key root.pub --unix-time 1750000000
```

`msign certificate issue`は運営・fixture用であり、一般利用者向けのCertificate取得手順では
ありません。

## Security

- Device AuthorizationはPKCE S256を使用し、Accounts指定のpoll intervalを守ります。
- verification URLへ載せる値は公開`code`だけです。device codeやtokenは載せません。
- application private key、refresh credential、payload、sourceをCloudへ送りません。
- CertificateのMCER形式、Issuer署名、Subject、Developer、scope、Capability、期限を
  ローカルで検証します。
- 署名後にheader、ustar、Certificate、manifest署名、payload size/digestを検証します。
- AppStore Reviewerは公開時にCertificateの最新statusを再確認する責務を持ちます。
