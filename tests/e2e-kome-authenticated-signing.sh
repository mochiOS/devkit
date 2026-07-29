#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/kome-auth-e2e.XXXXXX")"
account_server_pid=""
ca_server_pid=""

cleanup() {
  if [ -n "$account_server_pid" ]; then
    kill "$account_server_pid" 2>/dev/null || true
  fi
  if [ -n "$ca_server_pid" ]; then
    kill "$ca_server_pid" 2>/dev/null || true
  fi
  rm -rf "$work_dir"
}
trap cleanup EXIT

cd "$repo_dir"
if [ "${KOME_E2E_SKIP_BUILD:-0}" != 1 ]; then
  cargo build --bins >/dev/null
fi
export PATH="$repo_dir/target/debug:$PATH"
export KOME_CONFIG_HOME="$work_dir/config"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$work_dir/no-secret-service"

cd "$work_dir"
kome new Example --id com.example.application --vendor "Example Developer" >/dev/null
cd Example

account_server="$work_dir/account-server.pl"
account_port_file="$work_dir/account-server.port"
account_requests="$work_dir/account-requests.http"
cat > "$account_server" <<'PERL'
use strict;
use warnings;
use IO::Socket::INET;

my ($port_file, $request_file, $expected_requests) = @ARGV;
my $server = IO::Socket::INET->new(
    LocalAddr => '127.0.0.1',
    LocalPort => 0,
    Proto => 'tcp',
    Listen => 8,
    Reuse => 1,
) or die "failed to listen: $!";
my $port = $server->sockport;
open(my $port_fh, '>', $port_file) or die "failed to write port file: $!";
print {$port_fh} $port;
close($port_fh);

sub read_request {
    my ($client) = @_;
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
    return $request;
}

open(my $requests, '>', $request_file) or die "failed to write request log: $!";
for (my $index = 0; $index < $expected_requests; $index++) {
    my $client = $server->accept() or die "failed to accept: $!";
    my $request = read_request($client);
    print {$requests} "===== request $index =====\n$request\n";
    my ($method, $path) = $request =~ m{^([A-Z]+)\s+(\S+)\s+HTTP/};
    my $body;
    if ($method eq 'POST' && $path eq '/v1/device/authorization') {
        $body = '{"device_code":"device-secret","user_code":"ABCD-EFGH",'
            . '"verification_uri":"http://127.0.0.1:' . $port . '/device",'
            . '"verification_uri_complete":"http://127.0.0.1:' . $port
            . '/device?code=ABCD-EFGH","expires_in":60,"interval":1}';
    } elsif ($method eq 'POST' && $path eq '/v1/device/token') {
        $body = '{"access_token":"access-secret","refresh_credential":"refresh-1",'
            . '"session_id":"session-1"}';
    } elsif ($method eq 'POST' && $path eq '/v1/token/refresh') {
        $body = '{"access_token":"access-secret","refresh_credential":"refresh-rotated",'
            . '"session_id":"session-1"}';
    } elsif ($method eq 'GET' && $path eq '/v1/account') {
        $body = '{"account_id":"019f9e5ac6687902b0e72fe53abfbef0",'
            . '"account_name":"jine","device_name":"Linux · Kome CLI"}';
    } elsif ($method eq 'GET' && $path eq '/v1/developers') {
        $body = '{"developers":[{"developer_id":"019f9e5ac6687902b0e72fe53abfbef1",'
            . '"name":"Example Developer","membership_status":"active",'
            . '"developer_status":"verified","certificate_issuable":true}]}';
    } else {
        die "unexpected Accounts request: $method $path";
    }
    print {$client} "HTTP/1.1 200 OK\r\n";
    print {$client} "content-type: application/json\r\n";
    print {$client} "content-length: " . length($body) . "\r\n";
    print {$client} "connection: close\r\n\r\n$body";
    close($client);
}
close($requests);
close($server);
PERL

perl "$account_server" "$account_port_file" "$account_requests" 10 &
account_server_pid="$!"
for _ in $(seq 1 100); do
  [ -s "$account_port_file" ] && break
  sleep 0.05
done
test -s "$account_port_file"
accounts_base="http://127.0.0.1:$(cat "$account_port_file")/v1"

kome login --no-browser --accounts-api-base "$accounts_base" > "$work_dir/login.out"
grep -Fx 'http://'"127.0.0.1:$(cat "$account_port_file")"'/device?code=ABCD-EFGH' "$work_dir/login.out" >/dev/null
grep -Fx 'Logged in as jine.' "$work_dir/login.out" >/dev/null
if grep -E 'device-secret|access-secret|refresh-1' "$work_dir/login.out" >/dev/null; then
  echo "login output exposed a secret" >&2
  exit 1
fi
test ! -e credentials.json
test ! -e .git/credentials.json
test -f "$KOME_CONFIG_HOME/credentials.json"

msign key generate --private-key "$work_dir/root.key" --public-key "$work_dir/root.pub" >/dev/null

ca_server="$work_dir/ca-server.pl"
ca_port_file="$work_dir/ca-server.port"
ca_request="$work_dir/ca-request.http"
cat > "$ca_server" <<'PERL'
use strict;
use warnings;
use IO::Socket::INET;

my ($port_file, $request_file, $issuer_key, $issuer_file, $subject_key, $certificate_file) = @ARGV;
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

my @issue = (
    'msign', 'certificate', 'issue',
    '--issuer-key', $issuer_key,
    '--subject-public-key', $subject_key,
    '--developer-id', '019f9e5ac6687902b0e72fe53abfbef1',
    '--serial', '7',
    '--not-before', '1',
    '--not-after', '4102444800',
    '--scope', 'exact:com.example.application',
    '--output', $certificate_file,
);
system(@issue) == 0 or die "failed to issue fixture certificate";

sub base64_file {
    my ($path) = @_;
    open(my $fh, '<', $path) or die "failed to read $path: $!";
    binmode($fh);
    local $/;
    my $bytes = <$fh>;
    close($fh);
    require MIME::Base64;
    return MIME::Base64::encode_base64($bytes, '');
}
my $certificate = base64_file($certificate_file);
open(my $issuer_fh, '<', $issuer_file) or die "failed to read $issuer_file: $!";
local $/;
my $issuer = <$issuer_fh>;
close($issuer_fh);
$issuer =~ s/\s+//g;
my $body = '{"certificate_base64":"' . $certificate
    . '","issuer_public_key":"' . $issuer
    . '","developer_id":"019f9e5ac6687902b0e72fe53abfbef1"}';
print {$client} "HTTP/1.1 200 OK\r\n";
print {$client} "content-type: application/json\r\n";
print {$client} "content-length: " . length($body) . "\r\n";
print {$client} "connection: close\r\n\r\n$body";
close($client);
close($server);
PERL

perl "$ca_server" \
  "$ca_port_file" \
  "$ca_request" \
  "$work_dir/root.key" \
  "$work_dir/root.pub" \
  "$work_dir/Example/keys/application.pub" \
  "$work_dir/developer.cert" &
ca_server_pid="$!"
for _ in $(seq 1 100); do
  [ -s "$ca_port_file" ] && break
  sleep 0.05
done
test -s "$ca_port_file"
ca_base="http://127.0.0.1:$(cat "$ca_port_file")/v1"

test ! -e target/debug/entry.elf
test ! -e dist/Example-unsigned.mpkg
test ! -e keys/application.key
test ! -e keys/application.pub
kome sign \
  --accounts-api-base "$accounts_base" \
  --developer-ca-api-base "$ca_base" > "$work_dir/sign-first.out"
wait "$ca_server_pid"
ca_server_pid=""

test -f target/debug/entry.elf
test -f dist/Example-unsigned.mpkg
test -f keys/application.key
test -f keys/application.pub
if ! grep -Fx 'keys/application.key' .gitignore >/dev/null \
  && ! grep -Fx 'keys/*.key' .gitignore >/dev/null; then
  echo "application private key is not ignored by Git" >&2
  exit 1
fi
test -f keys/developer.cert
test -f keys/developer.issuer.pub
test -f dist/Example.mpkg
grep -Fx 'Account:     jine' "$work_dir/sign-first.out" >/dev/null
grep -Fx 'Developer:   019f9e5ac6687902b0e72fe53abfbef1' "$work_dir/sign-first.out" >/dev/null
grep -Fx 'Verified:    OK' "$work_dir/sign-first.out" >/dev/null
if grep -E 'access-secret|refresh-(1|rotated)' "$work_dir/sign-first.out" >/dev/null; then
  echo "sign output exposed an Account secret" >&2
  exit 1
fi

grep -F 'POST /v1/developers/019f9e5ac6687902b0e72fe53abfbef1/certificates/issue HTTP/1.1' "$ca_request" >/dev/null
grep -F '"package_id":"com.example.application"' "$ca_request" >/dev/null
grep -F '"capabilities":[]' "$ca_request" >/dev/null
if grep -E 'application\.key|refresh-(1|rotated)|entry\.elf|src/main\.kome' "$ca_request" >/dev/null; then
  echo "DeveloperCA request exposed forbidden project or credential data" >&2
  exit 1
fi

msign package verify \
  dist/Example.mpkg \
  --root-public-key "$work_dir/root.pub" \
  --unix-time "$(date +%s)" >/dev/null

kome sign \
  --accounts-api-base "$accounts_base" \
  --developer-ca-api-base "$ca_base" > "$work_dir/sign-second.out"
grep -Fx 'Verified:    OK' "$work_dir/sign-second.out" >/dev/null

wait "$account_server_pid"
account_server_pid=""
test "$(grep -c '^===== request ' "$account_requests")" -eq 10
grep -F '"code_challenge_method":"S256"' "$account_requests" >/dev/null
grep -F '"code_verifier":' "$account_requests" >/dev/null
grep -F '"refresh_credential":"refresh-1"' "$account_requests" >/dev/null
grep -F '"refresh_credential":"refresh-rotated"' "$account_requests" >/dev/null

credential_file="$KOME_CONFIG_HOME/credentials.json"
test -f "$credential_file"
if [ "$(stat -c '%a' "$credential_file")" != 600 ]; then
  echo "fallback credential file is not owner-only" >&2
  exit 1
fi

echo "authenticated Kome signing flow passed"
