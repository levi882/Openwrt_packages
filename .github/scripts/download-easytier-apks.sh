#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: download-easytier-apks.sh <feed-dir>}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pin() {
  python3 "$script_dir/read-release-pin.py" "$1"
}

tag="$(pin easytier.tag)"
version="$(pin easytier.version)"
archive="EasyTier-${tag}-x86_64-SNAPSHOT.zip"
archive_sha256="$(pin easytier.archive_sha)"
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
  "$(pin easytier.main_name)" \
  "$(pin easytier.main_sha)"

copy_and_check \
  "$(pin easytier.luci_name)" \
  "$(pin easytier.luci_sha)"

copy_and_check \
  "$(pin easytier.i18n_name)" \
  "$(pin easytier.i18n_sha)"
