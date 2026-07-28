# mochiOS developer kit

このリポジトリは、mochiOSアプリをMPKG v1として生成・署名・検証するための
開発者向けCLIを提供します。

## Tools

- `kome`: Kome projectの作成、build、MPKG生成、署名フローの入口
- `mpack`: `.pkg`互換packageとunsigned MPKG v1の生成器
- `msign`: Ed25519鍵、MCER Developer Certificate、MPKG署名・検証ツール
- `komeup`: Kome toolchain installer

## Standard MPKG Flow

Cloud/AppStore向けの標準フローはMPKG v1です。

```sh
kome new Example --id org.example.application --developer org.example.developer
cd Example
kome build
kome pack
kome key generate
```

`kome pack`は既定でunsigned MPKG v1を生成します。

```text
dist/Example-unsigned.mpkg
```

生成されるMPKGは32 byte headerと無圧縮ustar streamで構成されます。
`signatures/`は署名前には存在しなくても構いません。
`msign package verify`と`msign certificate obtain`は、mochiOS上の
`signature.service`と同じ256MiB上限を事前に適用します。

Developer CertificateはCloud ConsoleまたはDeveloperCA APIから取得します。
初期実装ではConsole併用が有効な運用です。

```text
keys/application.pub と dist/Example-unsigned.mpkg をConsoleへ渡す
Consoleから keys/developer.cert を取得する
```

CLIから取得する場合:

```sh
kome certificate obtain \
  --developer org.example.developer \
  --public-key keys/application.pub \
  --package dist/Example-unsigned.mpkg \
  --output keys/developer.cert
```

署名:

```sh
kome sign \
  dist/Example-unsigned.mpkg \
  --certificate keys/developer.cert \
  --key keys/application.key \
  --output dist/Example.mpkg
```

ローカル検証:

```sh
kome verify dist/Example.mpkg \
  --issuer-public-key root.pub \
  --unix-time 1750000000
```

成功したsigned MPKGはGitHub Releaseへassetとして配置できます。

## Guides

- [Kome package guide](docs/kome-packaging.md)
- [MPKG v1 guide](docs/mpkg-v1.md)
- [Developer key management](docs/developer-key-management.md)
- [Certificate obtain guide](docs/certificate-obtain.md)
- [Package signing guide](docs/package-signing.md)
- [AppStore publish guide](docs/appstore-publish.md)
- [legacy .pkg migration guide](docs/legacy-pkg-migration.md)

## Generated Files

`kome new`:

```text
Kome.toml
src/main.kome
assets/
.gitignore
```

`.gitignore`には`target/`、`dist/`、`keys/*.key`を追加します。
秘密鍵をGitへ追加しないでください。

`kome build`:

```text
target/debug/entry.elf
```

現時点の`kome build`はmock ELFを生成します。`komec`本体は未実装です。

`kome pack`:

```text
dist/<name>-unsigned.mpkg
target/mpkg-staging/manifest.toml
target/mpkg-staging/payload/bundle/...
```

`target/mpkg-staging`は中間生成物です。MPKG manifestには実payloadのsizeと
SHA-256 digestが入ります。

`kome key generate`:

```text
keys/application.key
keys/application.pub
```

鍵はEd25519です。秘密鍵はraw 32 byte signing keyのBase64、公開鍵はraw
32 byte verifying keyのBase64です。既存ファイルは上書きしません。
Unix環境では秘密鍵を可能な範囲でowner-only permissionで作成します。

`kome certificate obtain`:

```text
keys/developer.cert
```

保存前にMCER decode、Subject公開鍵、Subject Key ID、Package ID scope、
Capability許可、有効期間を確認します。`application.key`、MPKG payload、Kome sourceは
Cloudへ送信しません。

`kome sign`:

```text
dist/<name>.mpkg
```

MPKG内に次を追加します。

```text
signatures/developer.cert
signatures/manifest.sig
```

`manifest.sig`は次のbyte列への64 byte Ed25519署名です。

```text
"mochios-mpkg-manifest-v1\0" || SHA-256(manifest.toml bytes)
```

署名処理はmanifestとpayload bytesを変更しません。

## Legacy .pkg Compatibility

従来のKome `.pkg`フローは互換性のため残していますが、AppStore向けではありません。
MPKG v1とlegacy `.pkg`を自動判定で混ぜません。

```sh
kome pack --legacy
kome sign --legacy
kome verify --legacy
kome keygen
```

legacy署名は`.pkg`内の`META/signature.toml`を使用します。

```toml
version = 1
algorithm = "ed25519"
key_id = "application"
public_key = "..."
package_hash = "..."
signature = "..."
```

## Low-level Commands

Unsigned MPKG v1を直接作る場合:

```sh
mpack create \
  --manifest manifest.toml \
  --payload payload \
  --output app.mpkg
```

`payload` directoryの中身はMPKG内で`payload/`配下に入ります。
現行v1で受理するpayload rootは`root/`と`bundle/`です。

運営者、fixture、offline CA用途の証明書発行:

```sh
msign certificate issue \
  --issuer-key issuer.key \
  --subject-public-key application.pub \
  --developer-id org.example.developer \
  --serial 1 \
  --not-before 1700000000 \
  --not-after 1800000000 \
  --scope exact:org.example.application \
  --capability window.create \
  --output developer.cert
```

互換性のため`--root-key`と`--developer-key`も残していますが、
証明書発行者がDeveloper秘密鍵を読む必要はありません。

MPKG署名と検証:

```sh
msign package sign app.mpkg --certificate developer.cert --key application.key
msign package verify app.mpkg --issuer-public-key root.pub --unix-time 1750000000
```

## Security Notes

- Developer秘密鍵をCloudへ送らない
- Developer秘密鍵をstdoutへ出さない
- Developer秘密鍵をMPKGへ格納しない
- Certificate Subject公開鍵と`application.key`由来公開鍵を照合する
- Cloudから返ったCertificateはローカルでMCERとして検証してから保存する
- package scope外、Capability外、期限外のCertificateでは署名しない
- 既に署名済みのMPKGは既定で再署名しない
- path traversal、symlink、hard link、device、FIFO、PAX/GNU拡張を拒否する
- OS側の検証転送上限に合わせ、256MiBを超えるMPKGを拒否する

## Install

```sh
make install
```
