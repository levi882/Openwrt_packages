#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$repo_root/scripts/setup-restore-rust.sh"
manifest="$repo_root/packages/overlay-restore/src/Cargo.toml"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$restore_rust_root/target}"
cargo fmt --manifest-path "$manifest" -- --check
# Exercise replacement permissions with a restrictive umask.
(umask 077; cargo test --locked --manifest-path "$manifest" --target x86_64-unknown-linux-musl)
cargo clippy --locked --manifest-path "$manifest" --all-targets --target x86_64-unknown-linux-musl -- -D warnings
