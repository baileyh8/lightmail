#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=15.0
if [ -x .tooling/cargo/bin/cargo ]; then
  export CARGO_HOME="$PWD/.tooling/cargo"
  export RUSTUP_HOME="$PWD/.tooling/rustup"
  export PATH="$PWD/.tooling/cargo/bin:$PATH"
fi
exec cargo "$@"
