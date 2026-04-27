#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-rtp2httpd-apks.sh <feed-dir>}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pin() {
  python3 "$script_dir/read-release-pin.py" "$1"
}

tag="$(pin rtp2httpd.tag)"
version="$(pin rtp2httpd.version)"
release="$(pin rtp2httpd.package_release)"
base_url="https://github.com/stackia/rtp2httpd/releases/download/${tag}"

mkdir -p "$repo_dir"

download_and_check() {
  local source_file="$1"
  local sha256="$2"
  local dest_file="${3:-$source_file}"
  local tmp_file="${repo_dir}/.${source_file}.download"

  echo "Downloading ${source_file} -> ${dest_file}"
  curl -fL --retry 3 --retry-delay 2 -o "$tmp_file" "${base_url}/${source_file}"

  printf '%s  %s\n' "$sha256" "$tmp_file" | sha256sum -c -
  mv "$tmp_file" "${repo_dir}/${dest_file}"
}

download_and_check \
  "rtp2httpd-${version}-${release}_x86_64.apk" \
  "$(pin rtp2httpd.main_sha)" \
  "rtp2httpd-${version}-${release}.apk"

download_and_check \
  "luci-app-rtp2httpd-${version}-${release}.apk" \
  "$(pin rtp2httpd.luci_sha)"

download_and_check \
  "luci-i18n-rtp2httpd-zh-cn-${version}.apk" \
  "$(pin rtp2httpd.i18n_sha)"
