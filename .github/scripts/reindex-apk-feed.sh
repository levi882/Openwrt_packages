#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:?usage: reindex-apk-feed.sh <feed-dir>}"

[ -n "${PRIVATE_KEY:-}" ] || {
  echo "PRIVATE_KEY secret is required to sign packages.adb" >&2
  exit 1
}

[ -d "$repo_dir" ] || {
  echo "Feed directory not found: $repo_dir" >&2
  exit 1
}

repo_abs="$(realpath "$repo_dir")"
key_file="$(mktemp)"
cleanup() {
  rm -f "$key_file"
}
trap cleanup EXIT

printf '%s\n' "$PRIVATE_KEY" > "$key_file"
chmod 600 "$key_file"

docker run --rm \
  --entrypoint /bin/bash \
  -v "${repo_abs}:/repo" \
  -v "${key_file}:/tmp/private-key.pem:ro" \
  sdk \
  -lc '
    set -euo pipefail
    cd /repo
    shopt -s nullglob
    apks=(*.apk)
    [ "${#apks[@]}" -gt 0 ] || {
      echo "No APK files found in /repo" >&2
      exit 1
    }

    rm -f packages.adb
    /builder/staging_dir/host/bin/apk mkndx \
      --root /builder \
      --keys-dir /builder \
      --allow-untrusted \
      --sign /tmp/private-key.pem \
      --output packages.adb \
      "${apks[@]}"

    /builder/staging_dir/host/bin/apk adbdump --format json packages.adb >/dev/null
  '

test -s "${repo_dir}/packages.adb"
echo "Reindexed $(find "$repo_dir" -maxdepth 1 -name "*.apk" | wc -l) APK packages in ${repo_dir}"
