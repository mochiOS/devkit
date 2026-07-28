# Legacy .pkg Migration Guide

legacy `.pkg`は互換性のため残っています。

```sh
kome pack --legacy
kome sign --legacy
kome verify --legacy
kome keygen
```

legacy署名は`.pkg`内の`META/signature.toml`を使用します。

```toml
version = 1
algorithm = "ed25519"
key_id = "application"
public_key = "..."
package_hash = "..."
signature = "..."
```

MPKG v1とは自動判定で混ぜません。AppStore向けには次の標準フローへ移行します。

```sh
kome pack
kome key generate
kome certificate obtain
kome sign
kome verify
```
