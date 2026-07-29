# Legacy .pkg Migration Guide

legacy `.pkg`はAppStore向け標準形式ではありません。現在の`kome`標準フローは
MPKG v1のみを生成・署名・検証します。

既存の`.pkg`成果物は、MPKG v1として自動判定または自動変換しません。
AppStore向けには次の標準フローへ移行します。

```sh
kome pack
kome key generate
kome certificate obtain
kome sign
kome verify
```
