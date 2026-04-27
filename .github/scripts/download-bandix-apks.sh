#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-bandix-apks.sh <feed-dir>}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pin() {
  python3 "$script_dir/read-release-pin.py" "$1"
}

bandix_tag="$(pin bandix.tag)"
bandix_version="$(pin bandix.version)"
luci_tag="$(pin bandix.luci_tag)"
luci_version="$(pin bandix.luci_version)"
i18n_version="$(pin bandix.i18n_version)"

mkdir -p "$repo_dir"

download_and_check() {
  local url="$1"
  local source_file="$2"
  local sha256="$3"
  local dest_file="$4"
  local tmp_file="${repo_dir}/.${source_file}.download"

  echo "Downloading ${source_file} -> ${dest_file}"
  curl -fL --retry 3 --retry-delay 2 -o "$tmp_file" "$url"

  printf '%s  %s\n' "$sha256" "$tmp_file" | sha256sum -c -
  mv "$tmp_file" "${repo_dir}/${dest_file}"
}

i18n_apk_version="${i18n_version%.*}~${i18n_version##*.}"

download_and_check \
  "https://github.com/timsaya/openwrt-bandix/releases/download/${bandix_tag}/bandix-${bandix_version}_x86_64.apk" \
  "bandix-${bandix_version}_x86_64.apk" \
  "$(pin bandix.main_sha)" \
  "bandix-${bandix_version}.apk"

download_and_check \
  "https://github.com/timsaya/luci-app-bandix/releases/download/${luci_tag}/luci-app-bandix-${luci_version}_all.apk" \
  "luci-app-bandix-${luci_version}_all.apk" \
  "$(pin bandix.luci_sha)" \
  "luci-app-bandix-${luci_version}.apk"

download_and_check \
  "https://github.com/timsaya/luci-app-bandix/releases/download/${luci_tag}/luci-i18n-bandix-zh-cn-${i18n_version}_all.apk" \
  "luci-i18n-bandix-zh-cn-${i18n_version}_all.apk" \
  "$(pin bandix.i18n_sha)" \
  "luci-i18n-bandix-zh-cn-${i18n_apk_version}.apk"
