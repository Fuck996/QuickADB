#!/usr/bin/env bash
# Copyright (C) 2026 QuickADB contributors
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
    echo '请在 Apple 芯片 Mac 上使用此脚本。' >&2
    exit 1
fi
sdk_version=$(xcrun --sdk macosx --show-sdk-version)
if (( ${sdk_version%%.*} < 27 )); then
    echo "需要 macOS 27 SDK，当前为 $sdk_version。请在 Xcode 中选择对应版本的命令行工具。" >&2
    exit 1
fi

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"
source="$project_root/src/macos"
output="$project_root/artifacts/macos"
target="$project_root/target"
rust_target=aarch64-apple-darwin
export MACOSX_DEPLOYMENT_TARGET=27.0

version=$(awk -F '"' '/^version = "/ { print $2; exit }' Cargo.toml)
mkdir -p "$output"
stage=$(mktemp -d "$output/.build.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT
app="$stage/QuickADB.app"
contents="$app/Contents"
resources="$contents/Resources"
executable="$contents/MacOS/QuickADB"
mkdir -p "$resources" "$contents/MacOS"

# Xcode links the Rust engine and its bundled libusb into the app's only executable.
CARGO_PROFILE_RELEASE_STRIP=none cargo rustc --locked --release --lib --crate-type staticlib \
    --target "$rust_target" --target-dir "$target"
xcrun --sdk macosx swiftc \
    -target arm64-apple-macos27.0 -sdk "$(xcrun --sdk macosx --show-sdk-path)" \
    -swift-version 5 -O -whole-module-optimization -parse-as-library -module-name QuickADB \
    -I "$source/QuickADBBridge" "$source/"*.swift \
    -L "$target/$rust_target/release" -lquickadb \
    -framework IOKit -framework Security -framework CoreFoundation -framework SystemConfiguration \
    -o "$executable"

cp "$source/Info.plist" "$contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$version" "$contents/Info.plist"
plutil -replace CFBundleVersion -string "$version" "$contents/Info.plist"
if [[ -n "${QUICKADB_MAC_BETA:-}" ]]; then
    plutil -replace CFBundleDisplayName -string "QuickADB 测试版" "$contents/Info.plist"
fi
plutil -lint "$contents/Info.plist"
cp NOTICE LICENSE assets/ThirdPartyNotices.txt "$resources/"

iconset="$stage/AppIcon.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
    size2=$((size * 2))
    sips -z "$size" "$size" assets/AppIconDisplay.png \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z "$size2" "$size2" assets/AppIconDisplay.png \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$resources/AppIcon.icns"
# Trim the source icon to the visible alpha bounds before setting its menu-bar size.
xcrun --sdk macosx swift scripts/Prepare-MacTrayIcon.swift assets/TrayIcon.png "$resources/TrayIcon.png"

identity=${QUICKADB_SIGN_IDENTITY:--}
if [[ "$identity" == "-" ]]; then
    codesign --force --sign - "$app"
else
    codesign --force --sign "$identity" --options runtime --timestamp "$app"
fi
codesign --verify --strict "$app"

dependencies=$(otool -L "$executable")
printf '%s\n' "$dependencies"
if ! printf '%s\n' "$dependencies" | awk 'NR > 1 && $1 !~ /^\/(System\/Library\/|usr\/lib\/)/ { bad = 1 } END { exit bad }'; then
    echo 'QuickADB 链接了非系统动态库。' >&2
    exit 1
fi

result="$output/QuickADB.app"
if [[ -e "$result" ]]; then
    mv "$result" "$stage/Previous.app"
fi
if ! mv "$app" "$result"; then
    if [[ -e "$stage/Previous.app" ]]; then
        mv "$stage/Previous.app" "$result"
    fi
    exit 1
fi
printf '已构建 %s\n' "$result"
