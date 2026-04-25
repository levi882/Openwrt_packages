#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-bandix-apks.sh <feed-dir>}"
bandix_tag="${BANDIX_RELEASE_TAG:-v0.12.7}"
bandix_version="${BANDIX_VERSION:-0.12.7-r1}"
luci_tag="${LUCI_BANDIX_RELEASE_TAG:-v0.12.6}"
luci_version="${LUCI_BANDIX_VERSION:-0.12.6-r1}"
i18n_version="${LUCI_BANDIX_I18N_VERSION:-26.068.39505.1002c41}"

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
  "bce17c28dfd9c269facdbe3c91495435ebb2b677ef0f360b31c22d2cc0c049a2" \
  "bandix-${bandix_version}.apk"

download_and_check \
  "https://github.com/timsaya/luci-app-bandix/releases/download/${luci_tag}/luci-app-bandix-${luci_version}_all.apk" \
  "luci-app-bandix-${luci_version}_all.apk" \
  "f0da4b13a50a243a5d92d4ad7d39e71bbb3378dfbe1824b184f382d67f08430c" \
  "luci-app-bandix-${luci_version}.apk"

download_and_check \
  "https://github.com/timsaya/luci-app-bandix/releases/download/${luci_tag}/luci-i18n-bandix-zh-cn-${i18n_version}_all.apk" \
  "luci-i18n-bandix-zh-cn-${i18n_version}_all.apk" \
  "3df1cd2ef7a8a9c888cabc9d5c3f71dcb4719a486491b60158070c9e37d082b9" \
  "luci-i18n-bandix-zh-cn-${i18n_apk_version}.apk"
