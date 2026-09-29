#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if command -v cargo >/dev/null 2>&1; then
  cargo --version
  exit 0
fi
mkdir -p .tooling/cargo .tooling/rustup
if [ ! -x .tooling/cargo/bin/cargo ]; then
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o .tooling/rustup-init.sh
  env CARGO_HOME="$PWD/.tooling/cargo" RUSTUP_HOME="$PWD/.tooling/rustup" \
    sh .tooling/rustup-init.sh -y --profile minimal --no-modify-path --default-toolchain stable
fi
env CARGO_HOME="$PWD/.tooling/cargo" RUSTUP_HOME="$PWD/.tooling/rustup" .tooling/cargo/bin/cargo --version
