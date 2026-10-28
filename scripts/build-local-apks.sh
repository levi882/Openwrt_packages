#!/usr/bin/env bash
set -euo pipefail

repo_root="${RESTORE_SOURCE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
output_dir="${1:-$repo_root/bin/packages/x86_64/myfeed}"
mkdir -p "$output_dir"
output_abs="$(realpath "$output_dir")"

build_in_sdk() {
    local sdk_root="$1" source_root="$2" destination="$3"
    local -a package_names clean_targets compile_targets rust_make_args
    read -r -a package_names <<< "${LOCAL_PACKAGES:-overlay-restore luci-app-overlay-restore docker-guard luci-app-docker-exposure}"
    local name build_rust=0
    for name in "${package_names[@]}"; do
        case "$name" in
            overlay-restore) build_rust=1 ;;
            luci-app-overlay-restore|docker-guard|luci-app-docker-exposure) ;;
            *) echo "Unsupported local package: $name" >&2; return 1 ;;
        esac
    done
    export RESTORE_SOURCE_ROOT="$source_root"
    rust_make_args=()
    if [ "$build_rust" = 1 ]; then
        export RESTORE_RUST_ROOT="${RESTORE_RUST_ROOT:-$sdk_root/.restore-rust}"
        source "$source_root/scripts/setup-restore-rust.sh"
        cargo fetch --locked --manifest-path "$source_root/packages/overlay-restore/src/Cargo.toml" --target x86_64-unknown-linux-musl
        rust_make_args=(OVERLAY_RESTORE_CARGO="$OVERLAY_RESTORE_CARGO" OVERLAY_RESTORE_RUSTC="$OVERLAY_RESTORE_RUSTC" OVERLAY_RESTORE_CARGO_HOME="$OVERLAY_RESTORE_CARGO_HOME")
    fi
    cd "$sdk_root"
    local build_log="${RESTORE_BUILD_LOG:-$sdk_root/logs/overlay-restore-local.log}"
    mkdir -p "$(dirname "$build_log")"
    if ! {
        ./scripts/feeds update base packages luci
        ./scripts/feeds install -a -p base
        ./scripts/feeds install -a -p packages
        ./scripts/feeds install -a -p luci
    } > "$build_log" 2>&1; then
        tail -n 60 "$build_log"
        return 1
    fi
    mkdir -p package/local
    clean_targets=()
    compile_targets=()
    for name in "${package_names[@]}"; do
        mkdir -p "package/local/$name"
        rsync -a --delete --exclude='__pycache__' --exclude='target' "$source_root/packages/$name/" "package/local/$name/"
        printf '\nCONFIG_PACKAGE_%s=m\n' "$name" >> .config
        clean_targets+=("package/local/$name/clean")
        compile_targets+=("package/local/$name/compile")
    done
    make defconfig >> "$build_log" 2>&1
    make NO_DEPS=1 "${clean_targets[@]}" >> "$build_log" 2>&1
    # Rust and its locked dependencies are prepared above. Runtime dependencies
    # are supplied by firmware feeds rather than rebuilt in this SDK invocation.
    if ! make -j"${RESTORE_BUILD_JOBS:-2}" NO_DEPS=1 \
        "${rust_make_args[@]}" "${compile_targets[@]}" V=s >> "$build_log" 2>&1; then
        tail -n 80 "$build_log"
        return 1
    fi
    echo "SDK package build completed; log: $build_log"
    # Publish only this build's package versions, retaining unrelated feed APKs.
    for name in "${package_names[@]}"; do
        find "$destination" -maxdepth 1 -type f -name "$name-*.apk" -delete
        find bin/packages -type f -name "$name-*.apk" -exec cp '{}' "$destination/" \;
        test -n "$(find "$destination" -maxdepth 1 -name "$name-*.apk" -print -quit)"
    done
}

if [ -n "${SDK_DIR:-}" ]; then
    export FORCE_UNSAFE_CONFIGURE=1
    build_in_sdk "$(realpath "$SDK_DIR")" "$repo_root" "$output_abs"
else
    sdk_image="${SDK_IMAGE:-ghcr.io/openwrt/sdk:${OPENWRT_ARCH:-x86_64}-${OPENWRT_BRANCH:-openwrt-25.12}}"
    cache_dir="${SDK_CACHE_DIR:-$repo_root/.openwrt-sdk-cache/${OPENWRT_ARCH:-x86_64}-${OPENWRT_BRANCH:-openwrt-25.12}}"
    mkdir -p "$cache_dir"
    script_abs="$(realpath "${BASH_SOURCE[0]}")"
    export -f build_in_sdk
    docker pull "$sdk_image"
    docker run --rm --user 0:0 --entrypoint /bin/bash \
        -e LOCAL_PACKAGES="${LOCAL_PACKAGES:-overlay-restore luci-app-overlay-restore docker-guard luci-app-docker-exposure}" \
        -v "$repo_root:/repo:ro" -v "$(realpath "$cache_dir"):/sdk-cache" -v "$output_abs:/output" \
        -v "$script_abs:/build-local-apks.sh:ro" "$sdk_image" -lc '
            set -euo pipefail
            export FORCE_UNSAFE_CONFIGURE=1
            cd /sdk-cache
            [ -f rules.mk ] || bash /builder/setup.sh
            RESTORE_SOURCE_ROOT=/repo SDK_DIR=/sdk-cache bash /build-local-apks.sh /output
        '
fi
