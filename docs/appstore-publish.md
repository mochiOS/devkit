# AppStore Publish Guide

AppStoreへ提出するassetはsigned MPKGです。

```text
dist/Example.mpkg
```

提出前にローカル検証を通します。

```sh
kome verify dist/Example.mpkg \
  --issuer-public-key root.pub \
  --unix-time 1750000000
```

成功時に表示される主な情報:

```text
verified_package_id
developer_id
certificate_serial
subject_key_id
manifest_digest
package_digest
allowed_capability
```

秘密鍵や署名内部状態は表示しません。

GitHub Releaseへ公開する場合は、`dist/Example.mpkg`をrelease assetとして
配置します。現時点のdevkitはGitHubへtokenを保存しません。upload補助は将来の
`kome publish`で拡張する想定です。
