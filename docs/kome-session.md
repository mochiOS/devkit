# Kome Session Guide

現在のAccount sessionを確認します。

```sh
kome account
```

Komeは保存済みrefresh credentialで短時間のaccess tokenを取得し、Account情報を表示します。
access tokenはmemory内だけで扱い、永続保存しません。

Developer membership:

```sh
kome developer list
kome developer use 019f9e5ac6687902b0e72fe53abfbef1
```

`developer list`はAccountsが返したmembershipだけを表示します。`developer use`は入力IDが
実際にAccountのactiveかつverifiedな発行可能Developerであることを確認してから、defaultを
ユーザー設定へ保存します。

ログアウト:

```sh
kome logout
```

Cloud側CLI sessionを失効し、ローカルrefresh credentialとmetadataを削除します。Cloudへ
到達できない場合もローカルcredentialは削除し、Cloud失効失敗を警告します。

保存先はWindows Credential Manager、macOS Keychain、Linux Secret Serviceを優先します。
利用できない場合だけOSの設定directory配下にowner-only fileを使います。project directory、
`Kome.toml`、`.git`、環境変数へcredentialを永続保存しません。
