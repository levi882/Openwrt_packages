#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-nikki-apks.sh <feed-dir>}"
tag="${NIKKI_RELEASE_TAG:-v1.26.0}"
openwrt_release="${NIKKI_OPENWRT_RELEASE:-25.12}"
archive="nikki_x86_64-openwrt-${openwrt_release}.tar.gz"
archive_sha256="7c1b21c4eea6fa29dca580ede2c7777f7cf6ac6c89ac90b1ed0edc3af2e9a53c"
url="https://github.com/morytyann/OpenWrt-nikki/releases/download/${tag}/${archive}"
tmp_dir="$(mktemp -d)"

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

mkdir -p "$repo_dir"

echo "Downloading ${archive}"
curl -fL --retry 3 --retry-delay 2 -o "${tmp_dir}/${archive}" "$url"

(
  cd "$tmp_dir"
  printf '%s  %s\n' "$archive_sha256" "$archive" | sha256sum -c -
)

tar -xzf "${tmp_dir}/${archive}" -C "$tmp_dir" --wildcards '*.apk'

copy_and_check() {
  local file="$1"
  local sha256="$2"

  [ -s "${tmp_dir}/${file}" ] || {
    echo "Expected APK not found in ${archive}: ${file}" >&2
    exit 1
  }

  (
    cd "$tmp_dir"
    printf '%s  %s\n' "$sha256" "$file" | sha256sum -c -
  )

  cp "${tmp_dir}/${file}" "$repo_dir/"
}

copy_and_check \
  "luci-app-nikki-1.26.0-r1.apk" \
  "1da947b8c911149d64f9dfd137d73363d7b692e026be6cc45b327f354e4c9675"

copy_and_check \
  "luci-i18n-nikki-ru-26.100.26541~bc29251.apk" \
  "3d8291f8883beb220ef345d33d0ff1c499534e1ee18e4c7d5bcdd42c6ae63ec4"

copy_and_check \
  "luci-i18n-nikki-zh-cn-26.100.26541~bc29251.apk" \
  "1d9c847bdc819f42d6697a1b5c1c2b9241da6f844f7e9197f41a3d37b382c182"

copy_and_check \
  "luci-i18n-nikki-zh-tw-26.100.26541~bc29251.apk" \
  "974ac4ee5cd80514c24913e973f029f10fc09d949e85063eb98a773a4b671e59"

copy_and_check \
  "nikki-2026.04.08-r1.apk" \
  "bc1530588290b861f50ed03bd3f476504566549cf0a4046ac4955439d3c680b4"
