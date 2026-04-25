#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-easytier-apks.sh <feed-dir>}"
tag="${EASYTIER_RELEASE_TAG:-v2.6.1}"
version="${EASYTIER_VERSION:-2.6.1}"
archive="EasyTier-${tag}-x86_64-SNAPSHOT.zip"
archive_sha256="8d15ef6d62a1f393537f96bc09cbb4cd5ad950ef6a6394bfeb978a2fdd269d2f"
url="https://github.com/EasyTier/luci-app-easytier/releases/download/${tag}/${archive}"
tmp_dir="$(mktemp -d)"

command -v unzip >/dev/null 2>&1 || {
  echo "unzip is required to extract ${archive}" >&2
  exit 1
}

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

mkdir -p "$repo_dir"

echo "Downloading ${archive}"
curl -fL --retry 3 --retry-delay 2 -o "${tmp_dir}/${archive}" "$url"

(
  cd "$tmp_dir"
  printf '%s  %s\n' "$archive_sha256" "$archive" | sha256sum -c -
)

unzip -j "${tmp_dir}/${archive}" '*.apk' -d "${tmp_dir}/apks"

copy_and_check() {
  local file="$1"
  local sha256="$2"

  [ -s "${tmp_dir}/apks/${file}" ] || {
    echo "Expected APK not found in ${archive}: ${file}" >&2
    exit 1
  }

  (
    cd "${tmp_dir}/apks"
    printf '%s  %s\n' "$sha256" "$file" | sha256sum -c -
  )

  cp "${tmp_dir}/apks/${file}" "$repo_dir/"
}

copy_and_check \
  "easytier-2.6.1.apk" \
  "f0dec9a5963cc6ea7d06d4ad3ff3e5d4df347eddca3988770975bac7b5ddfa5a"

copy_and_check \
  "luci-app-easytier-2.6.0-r2.apk" \
  "b97134cf583476fb7530cd86a781e9ee76b5a5362edad457adb72645f812f39f"

copy_and_check \
  "luci-i18n-easytier-zh-cn-26.108.04894~f333f04.apk" \
  "7652194e939995ae31d330d874e60aaf9b25dc1d27c5cdb128f6e34bca0709e6"
