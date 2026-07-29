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

perl -0pi -e 's/required = \[\]/required = ["window.create"]/' Kome.toml

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
grep -Fx 'requires = ["window.create"]' unsigned.extract/manifest.toml >/dev/null
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
  --not-after 4102444800 \
  --scope exact:org.example.application \
  --capability window.create \
  --output issued.cert >/dev/null

certificate_response="$(printf '{"certificate_base64":"%s"}' "$(base64 -w0 issued.cert)")"
certificate_response_file="$work_dir/certificate-response.json"
certificate_request_file="$work_dir/certificate-request.http"
certificate_port_file="$work_dir/certificate-server.port"
certificate_server="$work_dir/certificate-server.pl"
printf '%s' "$certificate_response" > "$certificate_response_file"
cat > "$certificate_server" <<'PERL'
use strict;
use warnings;
use IO::Socket::INET;

my ($port_file, $request_file, $response_file) = @ARGV;
my $server = IO::Socket::INET->new(
    LocalAddr => '127.0.0.1',
    LocalPort => 0,
    Proto => 'tcp',
    Listen => 1,
    Reuse => 1,
) or die "failed to listen: $!";

open(my $port_fh, '>', $port_file) or die "failed to write port file: $!";
print {$port_fh} $server->sockport;
close($port_fh);

my $client = $server->accept() or die "failed to accept: $!";
my $request = '';
while (index($request, "\r\n\r\n") < 0) {
    my $chunk = '';
    my $read = sysread($client, $chunk, 4096);
    die "failed to read request: $!" unless defined $read;
    last if $read == 0;
    $request .= $chunk;
}
if ($request =~ /content-length:\s*(\d+)/i) {
    my $length = $1;
    my $body_start = index($request, "\r\n\r\n") + 4;
    while (length($request) - $body_start < $length) {
        my $chunk = '';
        my $read = sysread($client, $chunk, 4096);
        die "failed to read body: $!" unless defined $read;
        last if $read == 0;
        $request .= $chunk;
    }
}

open(my $request_fh, '>', $request_file) or die "failed to write request file: $!";
print {$request_fh} $request;
close($request_fh);

open(my $response_fh, '<', $response_file) or die "failed to read response file: $!";
local $/;
my $body = <$response_fh>;
close($response_fh);

print {$client} "HTTP/1.1 200 OK\r\n";
print {$client} "content-type: application/json\r\n";
print {$client} "content-length: " . length($body) . "\r\n";
print {$client} "connection: close\r\n";
print {$client} "\r\n";
print {$client} $body;
close($client);
PERL

perl "$certificate_server" "$certificate_port_file" "$certificate_request_file" "$certificate_response_file" &
certificate_server_pid="$!"
for _ in $(seq 1 100); do
  if [ -s "$certificate_port_file" ]; then
    break
  fi
  sleep 0.05
done
if [ ! -s "$certificate_port_file" ]; then
  echo "certificate fixture server did not start" >&2
  kill "$certificate_server_pid" 2>/dev/null || true
  exit 1
fi
certificate_api_base="http://127.0.0.1:$(cat "$certificate_port_file")"
kome certificate obtain \
  --developer org.example.developer \
  --public-key keys/application.pub \
  --output keys/developer.cert \
  --api-base "$certificate_api_base" \
  --bearer-token test-token \
  --idempotency-key devkit-e2e-certificate-1 >/dev/null
wait "$certificate_server_pid"

grep -F 'POST /developers/org.example.developer/certificates/issue HTTP/1.1' "$certificate_request_file" >/dev/null
grep -Fi 'authorization: Bearer test-token' "$certificate_request_file" >/dev/null
grep -Fi 'x-idempotency-key: devkit-e2e-certificate-1' "$certificate_request_file" >/dev/null
if grep -F '"developer_id"' "$certificate_request_file" >/dev/null; then
  echo "certificate obtain request unexpectedly contains developer_id in its body" >&2
  exit 1
fi
grep -F '"package_id":"org.example.application"' "$certificate_request_file" >/dev/null
grep -F '"capabilities":["window.create"]' "$certificate_request_file" >/dev/null
if grep -F 'application.key' "$certificate_request_file" >/dev/null; then
  echo "certificate obtain request unexpectedly contains the private key path" >&2
  exit 1
fi
if grep -F 'entry.elf' "$certificate_request_file" >/dev/null; then
  echo "certificate obtain request unexpectedly contains payload metadata" >&2
  exit 1
fi
cmp issued.cert keys/developer.cert

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

if [ -n "${APPSTORE_REVIEWER_DIR:-}" ]; then
  msign certificate inspect keys/developer.cert > certificate.inspect
  certificate_serial="$(sed -n 's/^serial_number: //p' certificate.inspect)"
  certificate_subject_key_id="$(sed -n 's/^subject_key_id: //p' certificate.inspect)"
  certificate_developer_id="$(sed -n 's/^developer_id: //p' certificate.inspect)"
  certificate_issuer_key_id="$(sed -n 's/^issuer_key_id: //p' certificate.inspect)"
  "$repo_dir/tests/run-appstore-reviewer.sh" \
    "$APPSTORE_REVIEWER_DIR" \
    "$PWD/dist/Example.mpkg" \
    "$PWD/root.pub" \
    "$PWD/keys/application.pub" \
    "$certificate_serial" \
    "$certificate_subject_key_id" \
    "$certificate_developer_id" \
    "$certificate_issuer_key_id"
fi

cp dist/Example.mpkg dist/Example-tampered.mpkg
printf x >> dist/Example-tampered.mpkg
if kome verify dist/Example-tampered.mpkg --issuer-public-key root.pub --unix-time 1800000000 >/dev/null 2>&1; then
  echo "tampered MPKG unexpectedly verified" >&2
  exit 1
fi

echo "e2e MPKG flow passed"
