#!/usr/bin/env bash
# Copyright (C) 2026 QuickADB contributors
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail
repo="${GITHUB_REPOSITORY:?}"
commit="${GITHUB_SHA:?}"
version=$(awk -F '"' '/^version = "/ { print $2; exit }' Cargo.toml)
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
win_tag="v$version"
ref="${GITHUB_REF_NAME:?}"
if [[ "$ref" == "$win_tag" ]]; then
    mac_tag="$win_tag-mac-beta.1"
else
    revision=${ref#"$win_tag-mac-beta."}
    [[ "$revision" =~ ^[1-9][0-9]*$ ]]
    mac_tag="$ref"
fi
test "$(gh api "repos/$repo/git/ref/heads/main" --jq .object.sha)" = "$commit"
test "$(gh api "repos/$repo/commits/$ref" --jq .sha)" = "$commit"

work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
dir=artifacts/release
mac_stem="$dir/QuickADB-${mac_tag#v}"
mac_files=("$mac_stem-arm64.dmg" "$mac_stem-arm64.dmg.sha256" "$mac_stem-source.zip" "$mac_stem-legal.zip")

if [[ "$ref" == "$win_tag" ]]; then
    win_files=("$dir/QuickADB-$version-x64.exe" "$dir/QuickADB-$version-source.zip" "$dir/QuickADB-$version-legal.zip")
    if ! gh api "repos/$repo/git/ref/tags/$mac_tag" > "$work/mac-tag.json"; then
        gh api --method POST "repos/$repo/git/refs" -f "ref=refs/tags/$mac_tag" -f "sha=$commit" > "$work/mac-tag.json"
    fi
    test "$(jq -r .object.sha "$work/mac-tag.json")" = "$commit"
fi

verify_assets() {
    local id="$1" file size digest
    shift
    gh api "repos/$repo/releases/$id/assets?per_page=100" > "$work/assets.json"
    test "$(jq length "$work/assets.json")" = "$#"
    for file in "$@"; do
        size=$(wc -c < "$file")
        digest=$(sha256sum "$file" | awk '{print $1}')
        jq -e --arg name "${file##*/}" --arg digest "sha256:$digest" --argjson size "$size" \
            'any(.[]; .name == $name and .state == "uploaded" and .size == $size and .digest == $digest)' \
            "$work/assets.json" >/dev/null
        printf '%s  %s  %s bytes\n' "$digest" "${file##*/}" "$size"
    done
}

prepare_release() {
    local tag="$1" prerelease="$2" notes="$3" file id
    shift 3
    for file in "$@"; do test -s "$file"; done
    cp "$notes" "$work/notes.md"
    {
        printf '\n### SHA-256\n\n'
        for file in "$@"; do
            printf '\x60%s\x60  \n\x60%s\x60\n\n' "${file##*/}" "$(sha256sum "$file" | awk '{print $1}')"
        done
    } >> "$work/notes.md"
    gh api --paginate "repos/$repo/releases?per_page=100" \
        --jq ".[] | select(.tag_name == \"$tag\")" > "$work/release.json"
    if [[ ! -s "$work/release.json" ]]; then
        gh api --method POST "repos/$repo/releases" -f "tag_name=$tag" \
            -f "name=QuickADB ${tag#v}" -F draft=true -F "prerelease=$prerelease" \
            -F "body=@$work/notes.md" > "$work/release.json"
    fi
    id=$(jq -er .id "$work/release.json")
    if [[ "$(jq -r .draft "$work/release.json")" == true ]]; then
        gh release upload "$tag" --repo "$repo" --clobber "$@"
    fi
    verify_assets "$id" "$@"
}

if [[ "$ref" == "$win_tag" ]]; then
    prepare_release "$win_tag" false "docs/releases/$version.md" "${win_files[@]}"
fi
prepare_release "$mac_tag" true "docs/releases/${mac_tag#v}.md" "${mac_files[@]}"

gh release edit "$mac_tag" --repo "$repo" --draft=false --latest=false
if [[ "$ref" == "$win_tag" ]]; then
    gh release edit "$win_tag" --repo "$repo" --draft=false --latest
fi
