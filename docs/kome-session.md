# Kome Session Guide

現在のAccount sessionを確認します。

```sh
kome account
```

Komeは保存済みrefresh tokenで短時間のaccess tokenを取得し、初回login時にcredential storeへ
保存したAccount情報を表示します。refresh responseにはAccount情報を要求しません。
access tokenはmemory内だけで扱い、永続保存しません。

Developer membership:

```sh
kome developer list
kome developer use 019f9e5ac6687902b0e72fe53abfbef1
```

`developer list`はDeveloperCAがaccess tokenに対して返したDeveloperだけを表示します。
`developer use`は入力IDがactiveかつverifiedな発行可能Developerであることを確認してから、
defaultをユーザー設定へ保存します。

ログアウト:

```sh
kome logout
```

Cloud側のcurrent CLI sessionを失効し、ローカルrefresh tokenとmetadataを削除します。Cloudへ
到達できない場合もローカルcredentialは削除し、Cloud失効失敗を警告します。

保存先はWindows Credential Manager、macOS Keychain、Linux Secret Serviceを優先します。
利用できない場合だけOSの設定directory配下にowner-only fileを使います。project directory、
`Kome.toml`、`.git`、環境変数へcredentialを永続保存しません。
