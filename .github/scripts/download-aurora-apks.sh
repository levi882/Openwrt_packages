#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-aurora-apks.sh <feed-dir>}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pin() {
  python3 "$script_dir/read-release-pin.py" "$1"
}

theme_tag="$(pin aurora.theme_tag)"
theme_version="$(pin aurora.theme_version)"
config_tag="$(pin aurora.config_tag)"
config_version="$(pin aurora.config_version)"
i18n_version="$(pin aurora.i18n_version)"

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
  "https://github.com/eamonxg/luci-theme-aurora/releases/download/${theme_tag}/luci-theme-aurora-${theme_version}.apk" \
  "luci-theme-aurora-${theme_version}.apk" \
  "$(pin aurora.theme_sha)" \
  "luci-theme-aurora-${theme_version}.apk"

download_and_check \
  "https://github.com/eamonxg/luci-app-aurora-config/releases/download/${config_tag}/luci-app-aurora-config-${config_version}.apk" \
  "luci-app-aurora-config-${config_version}.apk" \
  "$(pin aurora.config_sha)" \
  "luci-app-aurora-config-${config_version}.apk"

download_and_check \
  "https://github.com/eamonxg/luci-app-aurora-config/releases/download/${config_tag}/luci-i18n-aurora-config-zh-cn-${i18n_version}.apk" \
  "luci-i18n-aurora-config-zh-cn-${i18n_version}.apk" \
  "$(pin aurora.i18n_sha)" \
  "luci-i18n-aurora-config-zh-cn-${i18n_apk_version}.apk"
