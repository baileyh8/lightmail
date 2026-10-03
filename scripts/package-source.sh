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
# Keep cargo's Git-source mappings as well as crates.io. The archive is moved
# outside this checkout, so every vendored directory must be archive-relative.
python3 - "$STAGE/.cargo/config.toml" <<'PYTHON'
import re
import sys
from pathlib import Path
config = Path("build/vendor-config.toml").read_text()
config = re.sub(r'^directory = ".*"$', 'directory = "vendor"', config, flags=re.MULTILINE)
Path(sys.argv[1]).write_text(config)
PYTHON
git rev-parse HEAD > "$STAGE/SOURCE_COMMIT.txt"
tar -czf "dist/${NAME}.tar.gz" -C build "$NAME"
(cd dist && shasum -a 256 "${NAME}.tar.gz" >> SHA256SUMS.txt)
printf 'Packaged dist/%s.tar.gz\n' "$NAME"
