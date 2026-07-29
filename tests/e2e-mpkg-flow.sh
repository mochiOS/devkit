#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/mochios-devkit-e2e.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

check_mpkg_header() {
  local package="$1"
  local size
  size="$(stat -c '%s' "$package")"
  if [ "$size" -lt 32 ]; then
    echo "MPKG is smaller than the 32 byte header: $package" >&2
    exit 1
  fi

  local header
  header=($(od -An -v -tu1 -N32 "$package"))
  if [ "${#header[@]}" -ne 32 ]; then
    echo "failed to read complete MPKG header: $package" >&2
    exit 1
  fi
  if [ "${header[0]}" -ne 77 ] || [ "${header[1]}" -ne 80 ] || [ "${header[2]}" -ne 75 ] || [ "${header[3]}" -ne 71 ]; then
    echo "invalid MPKG magic: $package" >&2
    exit 1
  fi

  local major minor header_len compression flags tar_len expected_len
  major=$((header[4] | (header[5] << 8)))
  minor=$((header[6] | (header[7] << 8)))
  header_len=$((header[8] | (header[9] << 8)))
  compression="${header[10]}"
  flags="${header[11]}"
  tar_len=$((header[12] | (header[13] << 8) | (header[14] << 16) | (header[15] << 24) | (header[16] << 32) | (header[17] << 40) | (header[18] << 48) | (header[19] << 56)))
  expected_len=$((size - 32))
  if [ "$major" -ne 1 ] || [ "$minor" -ne 0 ] || [ "$header_len" -ne 32 ] || [ "$compression" -ne 0 ] || [ "$flags" -ne 0 ]; then
    echo "invalid MPKG v1 header fields: $package" >&2
    exit 1
  fi
  if [ "$tar_len" -ne "$expected_len" ]; then
    echo "MPKG tar stream length mismatch: $package" >&2
    exit 1
  fi
  for index in $(seq 20 31); do
    if [ "${header[$index]}" -ne 0 ]; then
      echo "MPKG reserved header byte is non-zero: $package" >&2
      exit 1
    fi
  done
}

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

check_mpkg_header dist/Example-unsigned.mpkg
tail -c +33 dist/Example-unsigned.mpkg | tar -tf - > unsigned.entries
grep -Fx manifest.toml unsigned.entries >/dev/null
grep -Fx payload/bundle/entry.elf unsigned.entries >/dev/null
if grep '^signatures/' unsigned.entries >/dev/null; then
  echo "unsigned MPKG unexpectedly contains signatures" >&2
  exit 1
fi
mkdir unsigned.extract
tail -c +33 dist/Example-unsigned.mpkg | tar -xf - -C unsigned.extract
grep -Fx 'format = 1' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'id = "org.example.application"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'name = "Example"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'version = "0.1.0"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'vendor = "org.example.developer"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'kind = "application"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'architecture = "x86_64"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'abi = "mochios-1"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'path = "$/entry.elf"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'mode = "0755"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'path = "/applications/Example.app/entry.elf"' unsigned.extract/manifest.toml >/dev/null
grep -Fx 'requires = []' unsigned.extract/manifest.toml >/dev/null
entry_size="$(wc -c < unsigned.extract/payload/bundle/entry.elf | tr -d ' ')"
entry_digest="$(sha256sum unsigned.extract/payload/bundle/entry.elf | awk '{print $1}')"
grep -Fx "size = ${entry_size}" unsigned.extract/manifest.toml >/dev/null
grep -Fx "digest = \"sha256:${entry_digest}\"" unsigned.extract/manifest.toml >/dev/null

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
  --capability window.create \
  --output keys/developer.cert >/dev/null

kome sign --unix-time 1800000000 >/dev/null
kome verify --issuer-public-key root.pub --unix-time 1800000000 > verify.out
grep -Fx "verified_package_id: org.example.application" verify.out >/dev/null
grep -Fx "developer_id: org.example.developer" verify.out >/dev/null
grep -Fx "certificate_serial: 1" verify.out >/dev/null
grep -E "^subject_key_id: [0-9a-f]{64}$" verify.out >/dev/null
grep -E "^manifest_digest: [0-9a-f]{64}$" verify.out >/dev/null
grep -E "^package_digest: [0-9a-f]{64}$" verify.out >/dev/null
grep -Fx "allowed_capability: window.create" verify.out >/dev/null

test -f dist/Example.mpkg
check_mpkg_header dist/Example.mpkg
tail -c +33 dist/Example.mpkg | tar -tf - > signed.entries
grep -Fx signatures/developer.cert signed.entries >/dev/null
grep -Fx signatures/manifest.sig signed.entries >/dev/null
mkdir signed.extract
tail -c +33 dist/Example.mpkg | tar -xf - -C signed.extract
cmp unsigned.extract/manifest.toml signed.extract/manifest.toml
cmp unsigned.extract/payload/bundle/entry.elf signed.extract/payload/bundle/entry.elf
cmp keys/developer.cert signed.extract/signatures/developer.cert
signature_size="$(wc -c < signed.extract/signatures/manifest.sig | tr -d ' ')"
if [ "$signature_size" != "64" ]; then
  echo "manifest.sig must be a 64 byte Ed25519 signature" >&2
  exit 1
fi

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
