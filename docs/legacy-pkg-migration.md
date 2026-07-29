# Legacy .pkg Migration Guide

legacy `.pkg`はAppStore向け標準形式ではありません。既存成果物をMPKG v1として自動判定、
自動変換しません。

Kome projectの設定とpayloadを移行した後、標準フローを実行してください。

```sh
kome login
kome keygen
kome sign
```

2回目以降は`kome sign`だけでbuild、pack、Certificate確認、署名、検証を行います。
