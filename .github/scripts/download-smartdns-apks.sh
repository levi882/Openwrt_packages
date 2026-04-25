#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-smartdns-apks.sh <feed-dir>}"
tag="${SMARTDNS_RELEASE_TAG:-Release47.1}"
version="${SMARTDNS_VERSION:-1.2025.11.09-1443}"

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
  "48a92a6290b7a7281b6269d08678eb89a64bc49b0dc4f49e7297e8d6e9dfda25" \
  "smartdns-${apk_version}.apk"

download_and_check \
  "luci-app-smartdns.${version}.all-luci-all.apk" \
  "162a653278ca627dbfb313b761aca685f2338ba1c03ab60c72291c881511a923" \
  "luci-app-smartdns-${apk_version}.apk"

download_and_check \
  "luci-app-smartdns-lite.${version}.all-luci-lite-all.apk" \
  "98b634ede7b60ebce266f8281785504ade27989e6867624875364837aaf2a8fe" \
  "luci-app-smartdns-lite-${apk_version}.apk"
