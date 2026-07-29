#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 8 ]; then
  echo "usage: $0 REVIEWER_DIR PACKAGE ROOT_PUBLIC_KEY DEVELOPER_PUBLIC_KEY SERIAL SUBJECT_KEY_ID DEVELOPER_ID ISSUER_KEY_ID" >&2
  exit 2
fi

reviewer_dir="$(realpath "$1")"
shift
if [ ! -f "$reviewer_dir/Cargo.toml" ]; then
  echo "AppStore Reviewer Cargo.toml not found: $reviewer_dir" >&2
  exit 2
fi

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runner_dir="$(mktemp -d "${TMPDIR:-/tmp}/mochios-reviewer-compat.XXXXXX")"
trap 'rm -rf "$runner_dir"' EXIT
mkdir -p "$runner_dir/src"
cp "$repo_dir/tests/appstore-reviewer/Cargo.toml.in" "$runner_dir/Cargo.toml.in"
cp "$repo_dir/tests/appstore-reviewer/main.rs" "$runner_dir/src/main.rs"
REVIEWER_DIR="$reviewer_dir" perl -pe 's{\@REVIEWER_DIR\@}{$ENV{REVIEWER_DIR}}g' \
  "$runner_dir/Cargo.toml.in" > "$runner_dir/Cargo.toml"

cargo run --quiet --manifest-path "$runner_dir/Cargo.toml" -- "$@"
