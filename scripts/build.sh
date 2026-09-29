#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p build Generated/FFI Sources/Lightmail/Generated dist
bash scripts/cargo.sh build --release --lib
bash scripts/cargo.sh run --features bindings --bin uniffi-bindgen -- generate --library target/release/liblightmail_core.dylib --language swift --out-dir build/bindings
cp build/bindings/lightmail_core.swift Sources/Lightmail/Generated/
cp build/bindings/lightmail_coreFFI.h Generated/FFI/
cp build/bindings/lightmail_coreFFI.modulemap Generated/FFI/module.modulemap
# Link the static archive explicitly so the .app has no project-relative dylib dependency.
swift build -c release -Xlinker "$PWD/target/release/liblightmail_core.a"
APP="dist/轻邮.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp .build/release/Lightmail "$APP/Contents/MacOS/Lightmail"
cp Resources/Info.plist "$APP/Contents/Info.plist"
if [ -f Resources/AppIcon.icns ]; then cp Resources/AppIcon.icns "$APP/Contents/Resources/"; fi
codesign --force --sign "${LIGHTMAIL_SIGNING_IDENTITY:--}" --identifier com.bailey.lightmail "$APP"
codesign --verify --deep --strict "$APP"
du -sh "$APP"
