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

既定のDeveloperCA endpointは次です。

```text
POST https://ca.mochios.org/v1/developers/<developer-id>/certificates/issue
Authorization: Bearer <short-lived token>
X-Idempotency-Key: <16-128 safe ASCII characters>
```

`X-Idempotency-Key`は未指定なら暗号学的乱数から生成します。同じ発行要求を明示的に
再試行する場合は`--idempotency-key`へ同じ値を指定します。この値は秘密鍵では
ありませんが、通常ログへは出しません。

devkitがCloudへ送る情報:

```text
subject_public_key
package_id
capabilities
```

`--developer`にはDeveloperCAのDeveloper record IDを指定します。この値はJSON bodyでは
なくendpoint pathに含めます。MCER内のDeveloper IDは別の
`certificate_developer_id`であり、Cloud応答の`developer_record_id`と`developer_id`を
それぞれrequest pathとMCERへ照合してから保存します。

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
