#!/usr/bin/env bash
# Copyright (C) 2026 QuickADB contributors
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

revision=${1:-1}
if [[ ! "$revision" =~ ^[1-9][0-9]*$ ]]; then
    echo '请指定正整数形式的测试版编号。' >&2
    exit 1
fi

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"
version=$(awk -F '"' '/^version = "/ { print $2; exit }' Cargo.toml)
release="$version-mac-beta.$revision"
output="$project_root/artifacts/macos"
app="$output/QuickADB.app"
filename="QuickADB-$release-arm64.dmg"

QUICKADB_MAC_BETA="$revision" bash scripts/Build-Mac.sh

stage=$(mktemp -d "$output/.dmg.XXXXXXXX")
mounted=false
cleanup() {
    if [[ "$mounted" == true ]] && ! hdiutil detach "$stage/mount" >/dev/null; then
        echo "镜像仍挂载在 $stage/mount，请先卸载。" >&2
        return 1
    fi
    rm -rf -- "$stage"
}
trap cleanup EXIT
mkdir "$stage/files" "$stage/mount"
ditto "$app" "$stage/files/QuickADB.app"
ln -s /Applications "$stage/files/Applications"

image="$stage/$filename"
hdiutil create -volname 'QuickADB 测试版' -srcfolder "$stage/files" -fs HFS+ -format UDZO "$image"
hdiutil verify "$image"
hdiutil attach "$image" -readonly -nobrowse -mountpoint "$stage/mount"
mounted=true
installed="$stage/mount/QuickADB.app"
test "$(readlink "$stage/mount/Applications")" = /Applications
test "$(plutil -extract CFBundleDisplayName raw "$installed/Contents/Info.plist")" = 'QuickADB 测试版'
codesign --verify --strict "$installed"
lipo -verify_arch arm64 "$installed/Contents/MacOS/QuickADB"
cmp "$app/Contents/MacOS/QuickADB" "$installed/Contents/MacOS/QuickADB"
hdiutil detach "$stage/mount"
mounted=false

(cd "$stage" && shasum -a 256 "$filename" > "$filename.sha256")
mv -f "$image" "$stage/$filename.sha256" "$output/"
printf '已验证安装镜像 %s\n' "$output/$filename"
