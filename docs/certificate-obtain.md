# Certificate Obtain Guide

Developer CertificateはMCER v1 wire bytesとして保存します。

Console併用:

```text
keys/application.pub
dist/Example-unsigned.mpkg
```

この2つをConsoleへ渡し、取得したcertificateを次へ保存します。

```text
keys/developer.cert
```

CLI obtain:

```sh
kome certificate obtain \
  --developer org.example.developer \
  --public-key keys/application.pub \
  --package dist/Example-unsigned.mpkg \
  --output keys/developer.cert
```

devkitがCloudへ送る情報:

```text
developer_id
subject_public_key
package_id
capabilities
```

devkitがCloudへ送らない情報:

```text
application.key
MPKG payload
Kome source code
```

保存前の検証:

```text
MCER decode
Subject公開鍵一致
Subject Key ID一致
Developer ID一致
Package ID scope
Capability許可
現在時刻でのnot_before / not_after
```

Subject公開鍵が`application.pub`と一致しないcertificateは保存しません。
