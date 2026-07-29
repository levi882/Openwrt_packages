#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: reindex-apk-feed.sh <feed-dir>}"
sdk_image="${SDK_IMAGE:-ghcr.io/openwrt/sdk:${OPENWRT_ARCH:-x86_64}-${OPENWRT_BRANCH:-openwrt-25.12}}"
sdk_cache_dir="${SDK_CACHE_DIR:-.openwrt-sdk-cache/${OPENWRT_ARCH:-x86_64}-${OPENWRT_BRANCH:-openwrt-25.12}}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
canonicalizer="${script_dir}/canonicalize-apk-filenames.py"

[ -n "${PRIVATE_KEY:-}" ] || {
  echo "PRIVATE_KEY secret is required to sign packages.adb" >&2
  exit 1
}

[ -d "$repo_dir" ] || {
  echo "Feed directory not found: $repo_dir" >&2
  exit 1
}

[ -f "$canonicalizer" ] || {
  echo "APK filename canonicalizer not found: $canonicalizer" >&2
  exit 1
}

repo_abs="$(realpath "$repo_dir")"
canonicalizer_abs="$(realpath "$canonicalizer")"
mkdir -p "$sdk_cache_dir"
sdk_cache_abs="$(realpath "$sdk_cache_dir")"
key_file="$(mktemp)"
cleanup() {
  rm -f "$key_file"
}
trap cleanup EXIT

printf '%s\n' "$PRIVATE_KEY" > "$key_file"
chmod 644 "$key_file"

docker pull "$sdk_image"

docker run --rm \
  --user 0:0 \
  --entrypoint /bin/bash \
  -v "${repo_abs}:/repo" \
  -v "${sdk_cache_abs}:/sdk-cache" \
  -v "${key_file}:/tmp/private-key.pem:ro" \
  -v "${canonicalizer_abs}:/usr/local/bin/canonicalize-apk-filenames.py:ro" \
  "$sdk_image" \
  -lc '
    set -euo pipefail
    apk_bin=/sdk-cache/staging_dir/host/bin/apk

    if [ ! -x "$apk_bin" ]; then
      echo "OpenWrt SDK cache miss, running setup.sh..."
      cd /sdk-cache
      bash /builder/setup.sh
    else
      echo "OpenWrt SDK cache hit: $apk_bin"
    fi

    cd /repo
    shopt -s nullglob
    apks=(*.apk)
    [ "${#apks[@]}" -gt 0 ] || {
      echo "No APK files found in /repo" >&2
      exit 1
    }

    python3 /usr/local/bin/canonicalize-apk-filenames.py "$apk_bin" "${apks[@]}"
    apks=(*.apk)

    rm -f packages.adb
    "$apk_bin" mkndx \
      --root /sdk-cache \
      --keys-dir /sdk-cache \
      --allow-untrusted \
      --pkgname-spec "\${name}-\${version}.apk" \
      --sign /tmp/private-key.pem \
      --output packages.adb \
      "${apks[@]}"

    "$apk_bin" adbdump --format json packages.adb >/dev/null
  '

if command -v sudo >/dev/null 2>&1; then
  sudo chown -R "$(id -u):$(id -g)" "$repo_abs" "$sdk_cache_abs" || true
fi

test -s "${repo_dir}/packages.adb"
echo "Reindexed $(find "$repo_dir" -maxdepth 1 -name "*.apk" | wc -l) APK packages in ${repo_dir}"
