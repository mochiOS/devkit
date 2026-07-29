# AppStore Publish Guide

通常の提出用assetは`kome sign`が生成したsigned MPKGです。

```sh
kome sign
```

```text
dist/Example.mpkg
```

Komeは最終pathへ置く前にローカル検証を完了します。追加で低レベル検証を実行する場合:

```sh
msign package verify \
  dist/Example.mpkg \
  --root-public-key keys/developer.issuer.pub
```

ローカル検証はCertificateの形式、Issuer署名、有効期間、Package ID scope、Capability、
manifest署名、payload size/digestを確認します。期限内の既存Certificateを利用したoffline
署名を許可できる場合でも、AppStore Reviewerは公開時にCertificateの最新statusを検証する
責務を持ちます。

AppStore repositoryを同時にcheckoutしているfixture環境では、同じsigned MPKGをReviewerへ
渡す相互検証も実行できます。

```sh
make test-e2e-appstore APPSTORE_REVIEWER_DIR=/path/to/AppStore/reviewer
```

devkitはGitHub tokenを保存せず、release uploadを自動実行しません。
