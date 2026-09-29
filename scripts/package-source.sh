#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(python3 -c 'import plistlib; print(plistlib.load(open("Resources/Info.plist","rb"))["CFBundleShortVersionString"])')
NAME="Lightmail-v${VERSION}-source"
STAGE="build/${NAME}"
if [ -e "$STAGE" ]; then
  printf 'Source staging directory already exists: %s\n' "$STAGE" >&2
  exit 1
fi
mkdir -p "$STAGE/.cargo" dist
git archive HEAD | tar -x -C "$STAGE"
bash scripts/cargo.sh vendor --locked "$STAGE/vendor" > build/vendor-config.toml
cat > "$STAGE/.cargo/config.toml" <<'CONFIG'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CONFIG
git rev-parse HEAD > "$STAGE/SOURCE_COMMIT.txt"
tar -czf "dist/${NAME}.tar.gz" -C build "$NAME"
(cd dist && shasum -a 256 "${NAME}.tar.gz" >> SHA256SUMS.txt)
printf 'Packaged dist/%s.tar.gz\n' "$NAME"
