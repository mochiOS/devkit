#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/mochios-devkit-e2e.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

cd "$repo_dir"
cargo build --bins >/dev/null

export PATH="$repo_dir/target/debug:$PATH"

cd "$work_dir"
kome new Example --id org.example.application --developer org.example.developer >/dev/null
cd Example

kome build >/dev/null
kome pack >/dev/null

test -f dist/Example-unsigned.mpkg
test -f target/mpkg-staging/manifest.toml
test -f target/mpkg-staging/payload/bundle/entry.elf

if tar -tf dist/Example-unsigned.mpkg >/dev/null 2>&1; then
  echo "unsigned MPKG must not be a raw tar stream" >&2
  exit 1
fi

tail -c +33 dist/Example-unsigned.mpkg | tar -tf - > unsigned.entries
grep -Fx manifest.toml unsigned.entries >/dev/null
grep -Fx payload/bundle/entry.elf unsigned.entries >/dev/null
if grep '^signatures/' unsigned.entries >/dev/null; then
  echo "unsigned MPKG unexpectedly contains signatures" >&2
  exit 1
fi

msign key generate --private-key root.key --public-key root.pub >/dev/null
kome key generate >/dev/null

if kome key generate >/dev/null 2>&1; then
  echo "kome key generate unexpectedly overwrote existing keys" >&2
  exit 1
fi

msign certificate issue \
  --issuer-key root.key \
  --subject-public-key keys/application.pub \
  --developer-id org.example.developer \
  --serial 1 \
  --not-before 1700000000 \
  --not-after 1900000000 \
  --scope exact:org.example.application \
  --output keys/developer.cert >/dev/null

kome sign --unix-time 1800000000 >/dev/null
kome verify --issuer-public-key root.pub --unix-time 1800000000 >/dev/null

test -f dist/Example.mpkg
tail -c +33 dist/Example.mpkg | tar -tf - > signed.entries
grep -Fx signatures/developer.cert signed.entries >/dev/null
grep -Fx signatures/manifest.sig signed.entries >/dev/null

if grep -aF "$(cat keys/application.key)" dist/Example.mpkg >/dev/null; then
  echo "signed MPKG contains the developer private key" >&2
  exit 1
fi

cp dist/Example.mpkg dist/Example-tampered.mpkg
printf x >> dist/Example-tampered.mpkg
if kome verify dist/Example-tampered.mpkg --issuer-public-key root.pub --unix-time 1800000000 >/dev/null 2>&1; then
  echo "tampered MPKG unexpectedly verified" >&2
  exit 1
fi

echo "e2e MPKG flow passed"
