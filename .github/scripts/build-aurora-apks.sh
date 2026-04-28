#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: build-aurora-apks.sh <feed-dir>}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
sdk_image="${SDK_IMAGE:-ghcr.io/openwrt/sdk:${OPENWRT_ARCH:-x86_64}-${OPENWRT_BRANCH:-openwrt-25.12}}"
sdk_cache_dir="${SDK_CACHE_DIR:-.openwrt-sdk-cache/${OPENWRT_ARCH:-x86_64}-${OPENWRT_BRANCH:-openwrt-25.12}}"

pin() {
  python3 "$script_dir/read-release-pin.py" "$1"
}

download_source() {
  local url="$1"
  local sha256="$2"
  local dest="$3"
  local tmp="${dest}.download"

  echo "Downloading ${url}"
  curl -fL --retry 3 --retry-delay 2 -o "$tmp" "$url"
  printf '%s  %s\n' "$sha256" "$tmp" | sha256sum -c -
  mv "$tmp" "$dest"
}

theme_ref="$(pin aurora.theme_ref)"
config_ref="$(pin aurora.config_ref)"
theme_sha="$(pin aurora.theme_source_sha)"
config_sha="$(pin aurora.config_source_sha)"

mkdir -p "$repo_dir" "$sdk_cache_dir"
repo_abs="$(realpath "$repo_dir")"
sdk_cache_abs="$(realpath "$sdk_cache_dir")"
tmp_dir="$(mktemp -d)"

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

download_source \
  "https://github.com/eamonxg/luci-theme-aurora/archive/${theme_ref}.tar.gz" \
  "$theme_sha" \
  "$tmp_dir/luci-theme-aurora.tar.gz"

download_source \
  "https://github.com/eamonxg/luci-app-aurora-config/archive/${config_ref}.tar.gz" \
  "$config_sha" \
  "$tmp_dir/luci-app-aurora-config.tar.gz"

docker pull "$sdk_image"

docker run --rm \
  --user 0:0 \
  --entrypoint /bin/bash \
  -v "${repo_abs}:/repo" \
  -v "${sdk_cache_abs}:/sdk-cache" \
  -v "${tmp_dir}:/sources:ro" \
  "$sdk_image" \
  -lc '
    set -euo pipefail

    if [ ! -x /sdk-cache/staging_dir/host/bin/apk ]; then
      echo "OpenWrt SDK cache miss, running setup.sh..."
      cd /sdk-cache
      bash /builder/setup.sh
    else
      echo "OpenWrt SDK cache hit"
    fi

    cd /sdk-cache
    rm -rf package/aurora
    mkdir -p package/aurora/luci-theme-aurora package/aurora/luci-app-aurora-config
    tar -xzf /sources/luci-theme-aurora.tar.gz \
      --strip-components=1 \
      -C package/aurora/luci-theme-aurora
    tar -xzf /sources/luci-app-aurora-config.tar.gz \
      --strip-components=1 \
      -C package/aurora/luci-app-aurora-config

    rm -f \
      bin/packages/*/*/luci-theme-aurora-*.apk \
      bin/packages/*/*/luci-app-aurora-config-*.apk \
      bin/packages/*/*/luci-i18n-aurora-config-zh-cn-*.apk

    make defconfig
    make package/luci-theme-aurora/compile package/luci-app-aurora-config/compile -j"$(nproc)" V=s

    shopt -s nullglob
    apks=(
      bin/packages/*/*/luci-theme-aurora-*.apk
      bin/packages/*/*/luci-app-aurora-config-*.apk
      bin/packages/*/*/luci-i18n-aurora-config-zh-cn-*.apk
    )
    [ "${#apks[@]}" -eq 3 ] || {
      echo "Expected 3 Aurora APKs, found ${#apks[@]}" >&2
      printf "  %s\n" "${apks[@]}" >&2
      exit 1
    }
    cp -f "${apks[@]}" /repo/
  '

if command -v sudo >/dev/null 2>&1; then
  sudo chown -R "$(id -u):$(id -g)" "$repo_abs" "$sdk_cache_abs" || true
fi

echo "Built Aurora APKs into ${repo_dir}"
