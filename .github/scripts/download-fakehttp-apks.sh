#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-fakehttp-apks.sh <feed-dir>}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pin() {
  python3 "$script_dir/read-release-pin.py" "$1"
}

tag="$(pin fakehttp.tag)"
openwrt_release="$(pin fakehttp.openwrt_release)"
version="$(pin fakehttp.version)"
package_release="$(pin fakehttp.package_release)"
i18n_release="$(pin fakehttp.i18n_release)"
prefix="fakehttp-openwrt-${openwrt_release}-x86_64"
base_url="https://github.com/levi882/FakeHTTP/releases/download/${tag}"

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
  "${prefix}-fakehttp-${version}-${package_release}.apk" \
  "$(pin fakehttp.main_sha)" \
  "fakehttp-${version}-${package_release}.apk"

download_and_check \
  "${prefix}-luci-app-fakehttp-${version}-${package_release}.apk" \
  "$(pin fakehttp.luci_sha)" \
  "luci-app-fakehttp-${version}-${package_release}.apk"

download_and_check \
  "${prefix}-luci-i18n-fakehttp-zh-cn-${version}-${i18n_release}.apk" \
  "$(pin fakehttp.i18n_sha)" \
  "luci-i18n-fakehttp-zh-cn-${version}-${i18n_release}.apk"
