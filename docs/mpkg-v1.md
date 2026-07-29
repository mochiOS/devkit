# MPKG v1 Guide

MPKG v1は32 byte headerと無圧縮ustar streamで構成されます。

```text
offset  size  value
0       4     "MPKG"
4       2     major version = 1
6       2     minor version = 0
8       2     header size = 32
10      1     compression = 0
11      1     flags = 0
12      8     tar stream length
20      12    reserved = 0
```

`mpack create`でunsigned MPKGを直接生成できます。

```sh
mpack create \
  --manifest manifest.toml \
  --payload payload \
  --output app.mpkg
```

生成直後のMPKGには`signatures/`がなくても構いません。

```text
manifest.toml
payload/root/...
payload/bundle/...
```

`mpack create`は同じ入力から同じbytesを生成するため、entry順序、uid、gid、
mtime、mode、tar header種別を固定します。symlink、hard link、device、FIFO、
PAX/GNU拡張、絶対path、`.`、`..`、backslash、NUL、重複path、
未知top-level entryは拒否対象です。

AppStore Reviewerの提出上限に合わせ、`msign package verify`と
`msign certificate obtain`は128MiBを超えるMPKGを拒否します。OS側の
`signature.service`は256MiBまで扱えますが、公開前検証ではより厳しいReviewer上限を
採用します。
