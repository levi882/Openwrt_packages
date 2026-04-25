#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-rtp2httpd-apks.sh <feed-dir>}"
tag="${RTP2HTTPD_RELEASE_TAG:-v3.11.0}"
version="${RTP2HTTPD_VERSION:-3.11.0}"
release="${RTP2HTTPD_PACKAGE_RELEASE:-r1}"
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
  "1c870a54502940cd893be5b10dcf7e17fb27ea924fa38a522e1f0dfbef4d10fd" \
  "rtp2httpd-${version}-${release}.apk"

download_and_check \
  "luci-app-rtp2httpd-${version}-${release}.apk" \
  "1d76e0d6e8c7ff77b6651f353f84a576fd2c35e5dcf1f26ad1aab201adc0ed05"

download_and_check \
  "luci-i18n-rtp2httpd-zh-cn-${version}.apk" \
  "712fe32b6d30b80f0719158f6c7252557c4dd496f0b420522c4701e45982286a"
