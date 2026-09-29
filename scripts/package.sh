#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
APP="dist/轻邮.app"
VERSION=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$APP/Contents/Info.plist")
ARCH=$(uname -m)
NAME="Lightmail-v${VERSION}-macos-${ARCH}"
STAGE="build/package-${VERSION}-${ARCH}"
mkdir -p "$STAGE" dist
ditto "$APP" "$STAGE/轻邮.app"
cp LICENSE THIRD_PARTY_NOTICES.md "$STAGE/"
printf 'Lightmail v%s\nSource and build instructions: https://github.com/baileyh8/lightmail/tree/v%s\nLicense: GPL-3.0-or-later. See LICENSE and THIRD_PARTY_NOTICES.md.\n' "$VERSION" "$VERSION" > "$STAGE/README.txt"
codesign --verify --deep --strict "$STAGE/轻邮.app"
ditto -c -k --sequesterRsrc "$STAGE" "dist/${NAME}.zip"
(cd dist && shasum -a 256 "${NAME}.zip" > SHA256SUMS.txt)
unzip -tq "dist/${NAME}.zip"
printf 'Packaged dist/%s.zip\n' "$NAME"
