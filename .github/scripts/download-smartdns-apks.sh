#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-smartdns-apks.sh <feed-dir>}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pin() {
  python3 "$script_dir/read-release-pin.py" "$1"
}

tag="$(pin smartdns.tag)"
version="$(pin smartdns.version)"

mkdir -p "$repo_dir"
apk_version="${version%-*}-r${version##*-}"

download_and_check() {
  local source_file="$1"
  local sha256="$2"
  local dest_file="$3"
  local url="https://github.com/pymumu/smartdns/releases/download/${tag}/${source_file}"
  local tmp_file="${repo_dir}/.${source_file}.download"

  echo "Downloading ${source_file} -> ${dest_file}"
  curl -fL --retry 3 --retry-delay 2 -o "$tmp_file" "$url"

  printf '%s  %s\n' "$sha256" "$tmp_file" | sha256sum -c -
  mv "$tmp_file" "${repo_dir}/${dest_file}"
}

download_and_check \
  "smartdns.${version}.x86_64-openwrt-all.apk" \
  "$(pin smartdns.main_sha)" \
  "smartdns-${apk_version}.apk"

download_and_check \
  "luci-app-smartdns.${version}.all-luci-all.apk" \
  "$(pin smartdns.luci_sha)" \
  "luci-app-smartdns-${apk_version}.apk"

download_and_check \
  "luci-app-smartdns-lite.${version}.all-luci-lite-all.apk" \
  "$(pin smartdns.lite_sha)" \
  "luci-app-smartdns-lite-${apk_version}.apk"
