# Developer Key Management

Kome projectのapplication鍵はEd25519です。

```sh
kome keygen
```

生成物:

```text
keys/application.key  raw 32-byte Ed25519 signing keyのBase64
keys/application.pub  raw 32-byte Ed25519 verifying keyのBase64
```

公開鍵は秘密鍵から導出され、保存後に鍵ペア一致を検証します。両方が既に存在して一致する
場合は成功として状態を表示します。片方だけ存在する場合、または一致しない場合は上書きせず
失敗します。

秘密鍵はstdoutへ出力されず、Unixではowner-only permissionで作成されます。
`keys/application.key`はprojectの`.gitignore`へ重複なく追加されます。秘密鍵をCloud、
MPKG、source repository、release assetへ含めないでください。

Certificate発行で送信するのは`application.pub`だけです。鍵を変更すると既存の
`keys/developer.cert`は再利用されず、次の`kome sign`で新しいCertificateを取得します。

低レベルの鍵生成器:

```sh
msign key generate \
  --private-key keys/application.key \
  --public-key keys/application.pub
```

通常は`.gitignore`処理と既存鍵検証を含む`kome keygen`を使用してください。
