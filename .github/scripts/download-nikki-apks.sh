#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-nikki-apks.sh <feed-dir>}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pin() {
  python3 "$script_dir/read-release-pin.py" "$1"
}

tag="$(pin nikki.tag)"
openwrt_release="$(pin nikki.openwrt_release)"
archive="nikki_x86_64-openwrt-${openwrt_release}.tar.gz"
archive_sha256="$(pin nikki.archive_sha)"
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

while IFS=$'\t' read -r file digest; do
  copy_and_check "$file" "$digest"
done < <(python3 "$script_dir/read-release-pin.py" --pairs nikki.apks)
