#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-fakehttp-apks.sh <feed-dir>}"
tag="${FAKEHTTP_RELEASE_TAG:-v0.9.18}"
openwrt_release="${FAKEHTTP_OPENWRT_RELEASE:-25.12.2}"
version="${FAKEHTTP_VERSION:-0.9.18}"
package_release="${FAKEHTTP_PACKAGE_RELEASE:-r2}"
i18n_release="${FAKEHTTP_I18N_RELEASE:-r1}"
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
  "7b8fc9647944f2690084258539708cf370d34e627117250f402d190d528af58e" \
  "fakehttp-${version}-${package_release}.apk"

download_and_check \
  "${prefix}-luci-app-fakehttp-${version}-${package_release}.apk" \
  "eef39ee5c34709b6386cc8a326ec01017f1e79664d035eb80a4e1bbf05e9ab8f" \
  "luci-app-fakehttp-${version}-${package_release}.apk"

download_and_check \
  "${prefix}-luci-i18n-fakehttp-zh-cn-${version}-${i18n_release}.apk" \
  "99c5f2ab3ec264169b0e756e0290c7637528dbebbc5773ddb914f5f2ec105439" \
  "luci-i18n-fakehttp-zh-cn-${version}-${i18n_release}.apk"
