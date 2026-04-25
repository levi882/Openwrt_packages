add_myfeed() {
    [ -f /etc/openwrt_release ] || return 0
    . /etc/openwrt_release

    case "$DISTRIB_RELEASE" in
        *"25.12"*) ;;
        *)
            log "SKIP myfeed: not OpenWrt 25.12 ($DISTRIB_RELEASE)"
            return 0
            ;;
    esac

    MY_BASE="https://openwrt-packages.pages.dev"
    MY_REPO="$MY_BASE/openwrt-25.12/$DISTRIB_ARCH/myfeed/packages.adb"

    mkdir -p /etc/apk/keys /etc/apk/repositories.d

    wget -O /etc/apk/keys/myfeed.pem "$MY_BASE/public-key.pem" >>"$LOG" 2>&1 || {
        log "WARNING: myfeed key download failed"
        rm -f /etc/apk/keys/myfeed.pem
        return 0
    }

    echo "$MY_REPO" > /etc/apk/repositories.d/00-myfeed.list
    log "Added myfeed: $MY_REPO"
}
