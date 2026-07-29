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

AppStore repositoryを同時にcheckoutしている開発環境では、同じsigned MPKGを
実Reviewerへ渡す相互検証も実行できます。

```sh
make test-e2e-appstore APPSTORE_REVIEWER_DIR=/path/to/AppStore/reviewer
```

このtargetはKome project作成からCloud互換Certificate取得fixture、署名、ローカル検証、
`mochios-mpkg-reviewer::inspect_mpkg`による受理までを1本のE2Eとして確認します。

GitHub Releaseへ公開する場合は、`dist/Example.mpkg`をrelease assetとして
配置します。現時点のdevkitはGitHubへtokenを保存しません。upload補助は将来の
`kome publish`で拡張する想定です。
