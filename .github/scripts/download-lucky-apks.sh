#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-lucky-apks.sh <feed-dir>}"
tag="${LUCKY_RELEASE_TAG:-v2.27.2}"
lucky_version="${LUCKY_VERSION:-2.27.2-r1}"
luci_version="${LUCI_LUCKY_VERSION:-2.2.2-r1}"
i18n_version="${LUCI_LUCKY_I18N_VERSION:-0}"
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
  "141d49448fff081fa9aaefcb44c2ef47f660e1153203a729a5861ff2e66474b7" \
  "lucky-${lucky_version}.apk"

download_and_check \
  "luci-app-lucky-${luci_version}_x86_64.apk" \
  "42822bf7457fb541a259c0ace945a3f0c19e701113c01d07f1d9afa3cb56a598" \
  "luci-app-lucky-${luci_version}.apk"

download_and_check \
  "luci-i18n-lucky-zh-cn-${i18n_version}_x86_64.apk" \
  "10e4e86ca9bd5fa3ade1f929b38596a2481148690be9e7615948593bb77bd93c" \
  "luci-i18n-lucky-zh-cn-${i18n_version}.apk"
