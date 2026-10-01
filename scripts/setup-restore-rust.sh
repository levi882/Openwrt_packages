#!/usr/bin/env bash
set -euo pipefail

restore_repo_root="${RESTORE_SOURCE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
restore_rust_version="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$restore_repo_root/rust-toolchain.toml")"
test -n "$restore_rust_version"
restore_rust_root="${RESTORE_RUST_ROOT:-$restore_repo_root/.restore-rust-cache}"
mkdir -p "$restore_rust_root"
restore_rust_root="$(realpath "$restore_rust_root")"
export CARGO_HOME="$restore_rust_root/cargo"
export RUSTUP_HOME="$restore_rust_root/rustup"
export RUSTUP_TOOLCHAIN="$restore_rust_version"
export PATH="$CARGO_HOME/bin:$PATH"

if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$restore_rust_root/rustup-init.sh"
    sh "$restore_rust_root/rustup-init.sh" -y --no-modify-path --profile minimal --default-toolchain "$restore_rust_version"
fi
rustup toolchain install "$restore_rust_version" --profile minimal --component rustfmt --component clippy
rustup target add x86_64-unknown-linux-musl --toolchain "$restore_rust_version"
OVERLAY_RESTORE_CARGO="$(rustup which --toolchain "$restore_rust_version" cargo)"
OVERLAY_RESTORE_RUSTC="$(rustup which --toolchain "$restore_rust_version" rustc)"
export OVERLAY_RESTORE_CARGO OVERLAY_RESTORE_RUSTC
export OVERLAY_RESTORE_CARGO_HOME="$CARGO_HOME"
rustc --version
