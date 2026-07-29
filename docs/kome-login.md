# Kome Login Guide

```sh
kome login
```

KomeはAccountsのDevice Authorizationを開始する前にPKCE verifierを生成し、S256
challengeだけを送ります。Accountsから返された`verification_uri_complete`を既定ブラウザで
開き、開けない場合はURLとuser codeを表示します。

表示するURLには公開`code`だけを含めます。device code、access token、refresh tokenを
URLやログへ出しません。ブラウザを開かずURLだけ表示する場合:

```sh
kome login --no-browser
```

KomeはAccounts指定のpoll intervalを守り、`slow_down`では5秒延長します。
`authorization_pending`以外の拒否、期限切れ、無効grantは成功として扱いません。pollは
Device Authorizationの期限で終了します。Ctrl+C時は取得途中の秘密を保存せず終了します。

成功後はtoken response内のAccount情報を使い、CLI refresh tokenを安全なcredential storeへ
保存します。Developer一覧は必要なときにDeveloperCAから取得します。Developer IDは公開
識別子であり、秘密として扱いません。
