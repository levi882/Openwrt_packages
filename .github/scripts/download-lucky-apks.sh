#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-lucky-apks.sh <feed-dir>}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pin() {
  python3 "$script_dir/read-release-pin.py" "$1"
}

tag="$(pin lucky.tag)"
lucky_version="$(pin lucky.version)"
luci_version="$(pin lucky.luci_version)"
i18n_version="$(pin lucky.i18n_version)"
base_url="https://github.com/levi882/luci-app-lucky/releases/download/${tag}"

mkdir -p "$repo_dir"

download_and_check() {
  local source_file="$1"
  local sha256="$2"
  local dest_file="$3"
  local tmp_file="${repo_dir}/.${source_file}.download"

  echo "Downloading ${source_file} -> ${dest_file}"
  curl -fL --retry 3 --retry-delay 2 -o "$tmp_file" "${base_url}/${source_file}"

  printf '%s  %s\n' "$sha256" "$tmp_file" | sha256sum -c -
  mv "$tmp_file" "${repo_dir}/${dest_file}"
}

download_and_check \
  "lucky-${lucky_version}_x86_64.apk" \
  "$(pin lucky.main_sha)" \
  "lucky-${lucky_version}.apk"

download_and_check \
  "luci-app-lucky-${luci_version}_x86_64.apk" \
  "$(pin lucky.luci_sha)" \
  "luci-app-lucky-${luci_version}.apk"

download_and_check \
  "luci-i18n-lucky-zh-cn-${i18n_version}_x86_64.apk" \
  "$(pin lucky.i18n_sha)" \
  "luci-i18n-lucky-zh-cn-${i18n_version}.apk"
