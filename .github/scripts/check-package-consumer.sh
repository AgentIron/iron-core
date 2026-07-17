#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
temp_dir="$(mktemp -d)"
consumer_dir="$temp_dir/consumer"
trap 'rm -rf "$temp_dir"' EXIT

cargo new --quiet --lib "$consumer_dir"
cargo add --quiet \
  --manifest-path "$consumer_dir/Cargo.toml" \
  iron-core \
  --path "$repo_root" \
  --features embedded-python
cargo build --manifest-path "$consumer_dir/Cargo.toml"
