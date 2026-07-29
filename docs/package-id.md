# Package ID Rules

Package IDは`org.mochios.*`に限定されません。

有効例:

```text
com.example.paint
io.github.username.tool
dev.tas0.volume
jp.example.application
org.mochios.binder
```

規則:

- 小文字ASCIIのみ
- `.`区切りで2 segment以上
- 各segmentは`a-z`、`0-9`、`-`のみ
- 空segmentは禁止
- segment先頭・末尾の`-`は禁止
- 先頭・末尾の`.`、`..`は禁止
- 全体は255 bytes以下

大文字、underscore、1 segmentだけのIDは拒否します。この検証はKome、MSign、MCERで
共有する`mochios-certificate` validatorを使用し、devkit独自の規則を持ちません。
