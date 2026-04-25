#!/bin/sh
set -e

BACKUP_FILE="$1"

[ -f "$BACKUP_FILE" ] || {
    echo "用法：restore_overlay.sh 备份文件"
    exit 1
}

echo "即将恢复：$BACKUP_FILE"
echo "最低限度清理：旧 kmod/旧 apk 状态 + SmartDNS 运行缓存"
echo "配置尽量保留，软件包只按白名单恢复"
echo "恢复后会优先添加你的 myfeed，再使用当前固件官方源"
echo "输入 YES 继续："
read CONFIRM
[ "$CONFIRM" = "YES" ] || exit 0

DISTFEEDS_BEFORE_RESTORE=/tmp/restore-distfeeds.before-restore.list
rm -f "$DISTFEEDS_BEFORE_RESTORE"
if [ -f /etc/apk/repositories.d/distfeeds.list ]; then
    cp /etc/apk/repositories.d/distfeeds.list "$DISTFEEDS_BEFORE_RESTORE"
fi

echo "校验备份包..."
gzip -t "$BACKUP_FILE"
tar -tzf "$BACKUP_FILE" >/dev/null

echo "清空并恢复 overlay..."
rm -rf /overlay/*
tar -xzf "$BACKUP_FILE" -C /

UP=/overlay/upper
mkdir -p "$UP/root/restore-meta"

[ -s "$DISTFEEDS_BEFORE_RESTORE" ] && \
    cp "$DISTFEEDS_BEFORE_RESTORE" "$UP/root/restore-meta/distfeeds.before-restore.list"

ALLOW_RE='^(luci-app-(smartdns|smartdns-lite|dockerman|lucky|vlmcsd|fakehttp|watchcat|oaf|nikki|omcproxy|rtp2httpd|samba4|webdav|easytier|bandix|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|ota|diskman|ttyd|commands|quickfile|filebrowser|filemanager|fileassistant|ramfree|autoreboot|timedreboot|aurora-config)|luci-i18n-(smartdns|smartdns-lite|fakehttp|lucky|easytier|rtp2httpd|bandix|nikki|vlmcsd|watchcat|oaf|samba4|webdav|omcproxy|dockerman|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|ota|diskman|ttyd|commands|quickfile|filebrowser|filemanager|fileassistant|ramfree|autoreboot|timedreboot|aurora-config)-zh-cn|luci-i18n-app-omcproxy-zh-cn|luci-theme-aurora|python3|python3-requests|tcpdump|curl|bash)$'
SKIPPED_BY_ALLOW="$UP/root/restore-meta/world.skipped-by-allowlist"
PRUNED_BY_ALLOW="$UP/root/restore-meta/packages.pruned-by-allowlist"
CURRENT_WORLD="$UP/root/restore-meta/world.from-current-rom"
LUCI_BAD_OVERLAY="$UP/root/restore-meta/luci-bad-overlay.tar.gz"
: > "$SKIPPED_BY_ALLOW"
: > "$PRUNED_BY_ALLOW"
: > "$CURRENT_WORLD"

if [ -f /rom/etc/apk/world ]; then
    sed -E '
        /^kernel=/d;
        /^kmod-/d;
        /^$/d;
        /^#/d;
        s/[<>=~].*$//
        s/@.*$//
    ' /rom/etc/apk/world | sort -u > "$CURRENT_WORLD"
fi

echo "保存旧软件包清单，过滤 kernel=/kmod-*，并只保留白名单包..."
if [ -f "$UP/etc/apk/world" ]; then
    cp "$UP/etc/apk/world" "$UP/root/restore-meta/world.from-backup"

    sed -E '
        /^kernel=/d;
        /^kmod-/d;
        /^$/d;
        /^#/d;
        s/[<>=~].*$//
    ' "$UP/etc/apk/world" | while IFS= read -r PKG; do
        [ -n "$PKG" ] || continue
        BASE="${PKG%@*}"
        if echo "$BASE" | grep -Eq "$ALLOW_RE"; then
            echo "$PKG"
        else
            echo "$BASE" >> "$SKIPPED_BY_ALLOW"
        fi
    done | sort -u > "$UP/root/apk-world.restore-list"

    if [ -s "$SKIPPED_BY_ALLOW" ]; then
        sort -u "$SKIPPED_BY_ALLOW" > "$SKIPPED_BY_ALLOW.tmp"
        mv "$SKIPPED_BY_ALLOW.tmp" "$SKIPPED_BY_ALLOW"
    fi
fi

sanitize_luci_overlay() {
    echo "清理旧 LuCI overlay 运行时文件..."

    tar -czf "$LUCI_BAD_OVERLAY" \
        -C "$UP" \
        etc/config/luci \
        etc/board.json \
        usr/lib/lua/luci \
        usr/share/luci \
        www/luci-static \
        www/cgi-bin/luci \
        usr/share/rpcd/acl.d \
        usr/libexec/rpcd/luci \
        2>/dev/null || true

    rm -rf "$UP/usr/lib/lua/luci"
    rm -rf "$UP/usr/share/luci"
    rm -rf "$UP/www/luci-static"
    rm -rf "$UP/www/cgi-bin/luci"
    rm -rf "$UP/usr/share/rpcd/acl.d"/luci*
    rm -rf "$UP/usr/libexec/rpcd/luci"
    rm -f "$UP/etc/config/luci"
    rm -f "$UP/etc/board.json"

    # Old overlay backups may also carry whiteouts/opaque markers that still
    # hide the ROM's LuCI files after reboot. Remove the common ones too.
    rm -f "$UP/etc/.wh.board.json"
    rm -f "$UP/etc/config/.wh.luci"
    rm -f "$UP/usr/lib/lua/.wh.luci"
    rm -f "$UP/usr/share/.wh.luci"
    rm -f "$UP/www/.wh.luci-static"
    rm -f "$UP/www/cgi-bin/.wh.luci"
    rm -f "$UP/usr/libexec/rpcd/.wh.luci"
    rm -f "$UP/usr/share/rpcd/acl.d"/.wh.luci*
    rm -f "$UP/usr/lib/lua/.wh..wh..opq"
    rm -f "$UP/usr/share/.wh..wh..opq"
    rm -f "$UP/www/.wh..wh..opq"
    rm -f "$UP/www/cgi-bin/.wh..wh..opq"
    rm -f "$UP/usr/share/rpcd/acl.d/.wh..wh..opq"
    rm -f "$UP/usr/libexec/rpcd/.wh..wh..opq"
    rm -f "$UP/etc/.wh..wh..opq"
    rm -f "$UP/etc/config/.wh..wh..opq"
}

sanitize_luci_overlay

prune_non_allowlist_overlay_files() {
    DB="$UP/lib/apk/db/installed"
    [ -f "$DB" ] || return 0

    if [ ! -s "$CURRENT_WORLD" ]; then
        echo "未找到当前固件 world，跳过旧包文件清理"
        return 0
    fi

    echo "清理旧 overlay 中非当前固件且非白名单的软件包文件..."

    awk -v allow="$ALLOW_RE" '
        FNR == NR {
            current[$0] = 1
            next
        }
        /^P:/ {
            pkg = substr($0, 3)
            if (pkg !~ allow && !(pkg in current)) {
                print pkg
            }
        }
    ' "$CURRENT_WORLD" "$DB" | sort -u > "$PRUNED_BY_ALLOW"

    awk -v allow="$ALLOW_RE" '
        FNR == NR {
            current[$0] = 1
            next
        }
        /^P:/ {
            pkg = substr($0, 3)
            prune = (pkg !~ allow && !(pkg in current))
            dir = ""
            next
        }
        /^F:/ {
            dir = substr($0, 3)
            next
        }
        /^R:/ && prune && dir != "" {
            print dir "/" substr($0, 3)
        }
    ' "$CURRENT_WORLD" "$DB" | sort -u | while IFS= read -r REL; do
        [ -n "$REL" ] || continue
        case "$REL" in
            /*|../*|*/../*|..|*/..) continue ;;
            etc/config/*) continue ;;
            etc/smartdns/*) continue ;;
            etc/nikki/*) continue ;;
            etc/mihomo/*) continue ;;
            usr/share/nikki/*) continue ;;
            usr/share/mihomo/*) continue ;;
            root/.config/nikki/*) continue ;;
            root/.config/mihomo/*) continue ;;
        esac
        rm -f "$UP/$REL"
    done
}

prune_non_allowlist_overlay_files

echo "备份旧自定义源，并过滤会重建的源..."
if [ -f "$UP/etc/apk/repositories.d/customfeeds.list" ]; then
    grep -Ev 'core3\.cooluc\.com|levi882\.github\.io/Openwrt_packages|openwrt-packages\.pages\.dev|nikkinikki\.pages\.dev|^[[:space:]]*#|^[[:space:]]*$' \
        "$UP/etc/apk/repositories.d/customfeeds.list" > "$UP/root/restore-meta/customfeeds.list" || true
fi

echo "最低限度删除旧内核模块和旧 apk 状态..."
rm -rf "$UP/lib/modules"
rm -rf "$UP/lib/apk"
rm -rf "$UP/etc/apk"
rm -rf "$UP/etc/modules.d"

echo "清理 SmartDNS 运行缓存..."
rm -f "$UP/etc/smartdns/smartdns.cache"
rm -f "$UP/etc/smartdns/data"/smartdns.cache 2>/dev/null || true

rm -rf /overlay/work /overlay/lost+found

cat > "$UP/root/post_restore_reinstall.sh" <<'POSTEOF'
#!/bin/sh

LOG=/root/post_restore_reinstall.log
FAILED=/root/apk-install-failed.txt
NOT_IN_REPO=/root/apk-not-in-repo.txt
LIST=/root/apk-world.restore-list
APK_INDEX=/tmp/restore-apk-list.txt
APP_CONFIG_STASH=/root/restore-meta/app-config-stash.tar.gz
SERVICE_STATE=/root/restore-meta/network-addon-services.state
APK_ADD_BATCH_TIMEOUT="${APK_ADD_BATCH_TIMEOUT:-300}"
APK_ADD_LUCI_TIMEOUT="${APK_ADD_LUCI_TIMEOUT:-300}"
APK_ADD_RETRY_TIMEOUT="${APK_ADD_RETRY_TIMEOUT:-120}"
APK_ADD_ATTEMPTS="${APK_ADD_ATTEMPTS:-3}"
APK_ADD_RETRY_SLEEP="${APK_ADD_RETRY_SLEEP:-5}"
APK_ADD_CORE_CHUNK_SIZE="${APK_ADD_CORE_CHUNK_SIZE:-6}"
APK_RETRY_LUCI="${APK_RETRY_LUCI:-0}"
OPENWRT_MIRROR_BASE="${OPENWRT_MIRROR_BASE:-https://mirrors.cloud.tencent.com/openwrt}"
ALLOW_RE='^(luci-app-(smartdns|smartdns-lite|dockerman|lucky|vlmcsd|fakehttp|watchcat|oaf|nikki|omcproxy|rtp2httpd|samba4|webdav|easytier|bandix|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|ota|diskman|ttyd|commands|quickfile|filebrowser|filemanager|fileassistant|ramfree|autoreboot|timedreboot|aurora-config)|luci-i18n-(smartdns|smartdns-lite|fakehttp|lucky|easytier|rtp2httpd|bandix|nikki|vlmcsd|watchcat|oaf|samba4|webdav|omcproxy|dockerman|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|ota|diskman|ttyd|commands|quickfile|filebrowser|filemanager|fileassistant|ramfree|autoreboot|timedreboot|aurora-config)-zh-cn|luci-i18n-app-omcproxy-zh-cn|luci-theme-aurora|python3|python3-requests|tcpdump|curl|bash)$'
DROP_LUCI_RE='^luci-(app|i18n)-(wolplus|zerotier|sqm|socat|qbittorrent|passwall|passwall2|openlist|openlist2|natmap|nlbwmon|mosdns|homeproxy|frpc|eqos|argon-config|airplay2|airconnect|usb-printer|mentohust|ddns|ssr-plus|openclash)(-|$)|^luci-proto-wireguard$|^luci-i18n-proto-wireguard-'
MYFEED_RE='^(lucky|luci-app-lucky|luci-i18n-lucky-zh-cn|easytier|luci-app-easytier|luci-i18n-easytier-zh-cn|rtp2httpd|luci-app-rtp2httpd|luci-i18n-rtp2httpd-zh-cn|fakehttp|luci-app-fakehttp|luci-i18n-fakehttp-zh-cn|smartdns|luci-app-smartdns|luci-app-smartdns-lite|bandix|luci-app-bandix|luci-i18n-bandix-zh-cn|nikki|luci-app-nikki|luci-i18n-nikki-ru|luci-i18n-nikki-zh-cn|luci-i18n-nikki-zh-tw)$'

log() {
    echo "$@" | tee -a "$LOG"
}

stash_app_configs() {
    mkdir -p /root/restore-meta
    rm -f "$APP_CONFIG_STASH"

    set --
    for P in \
        etc/config/smartdns etc/smartdns \
        etc/config/nikki \
        etc/nikki/mixin.yaml etc/nikki/mixin.yaml.apk-new \
        etc/nikki/subscriptions etc/nikki/scripts etc/nikki/nftables \
        etc/nikki/profiles etc/nikki/proxy-providers \
        etc/nikki/rule-providers etc/nikki/rules \
        etc/nikki/providers etc/nikki/config.yaml \
        etc/config/fakehttp etc/config/appfilter \
        etc/config/omcproxy etc/config/rtp2httpd \
        etc/config/bandix etc/config/easytier \
        etc/config/dockerd etc/config/samba4 \
        etc/config/ttyd etc/config/lucky \
        etc/config/vlmcsd etc/config/watchcat \
        etc/config/webdav etc/config/upnpd
    do
        [ -e "/$P" ] && set -- "$@" "$P"
    done

    [ "$#" -gt 0 ] || return 0

    log "暂存应用配置，避免软件重装覆盖..."
    tar -czf "$APP_CONFIG_STASH" -C / "$@" >>"$LOG" 2>&1 || \
        log "WARNING: app config stash failed"
}

restore_app_configs() {
    [ -s "$APP_CONFIG_STASH" ] || return 0

    log "还原应用配置快照..."
    tar -xzf "$APP_CONFIG_STASH" -C / >>"$LOG" 2>&1 || \
        log "WARNING: app config restore failed"

    rm -f /etc/smartdns/smartdns.cache 2>/dev/null || true
    rm -f /etc/smartdns/data/smartdns.cache 2>/dev/null || true
}

service_enabled() {
    [ -x "/etc/init.d/$1" ] && /etc/init.d/"$1" enabled >/dev/null 2>&1
}

quiesce_network_addons() {
    mkdir -p /root/restore-meta
    : > "$SERVICE_STATE"

    for SVC in smartdns nikki fakehttp appfilter oaf; do
        [ -x "/etc/init.d/$SVC" ] || continue

        if service_enabled "$SVC"; then
            STATE=enabled
        else
            STATE=disabled
        fi

        echo "$SVC $STATE" >> "$SERVICE_STATE"
        log "临时停止网络增强服务: $SVC ($STATE)"
        /etc/init.d/"$SVC" stop 2>/dev/null || true
        /etc/init.d/"$SVC" disable 2>/dev/null || true
    done
}

restore_network_addons() {
    [ -s "$SERVICE_STATE" ] || return 0

    while read SVC STATE; do
        [ -n "$SVC" ] || continue
        [ -x "/etc/init.d/$SVC" ] || continue

        if [ "$STATE" = "enabled" ]; then
            log "恢复网络增强服务: $SVC"
            /etc/init.d/"$SVC" enable 2>/dev/null || true
            /etc/init.d/"$SVC" restart 2>/dev/null || \
                /etc/init.d/"$SVC" start 2>/dev/null || true
        else
            /etc/init.d/"$SVC" disable 2>/dev/null || true
            /etc/init.d/"$SVC" stop 2>/dev/null || true
        fi
    done < "$SERVICE_STATE"
}

restore_distfeeds() {
    mkdir -p /etc/apk/repositories.d

    if [ -s /root/restore-meta/distfeeds.before-restore.list ]; then
        cp /root/restore-meta/distfeeds.before-restore.list /etc/apk/repositories.d/distfeeds.list
        log "使用恢复前保存的 OpenWrt 软件源"
        return 0
    fi

    if [ -f /rom/etc/apk/repositories.d/distfeeds.list ]; then
        cp /rom/etc/apk/repositories.d/distfeeds.list /etc/apk/repositories.d/distfeeds.list
    else
        return 0
    fi

    [ -n "$OPENWRT_MIRROR_BASE" ] || return 0
    MIRROR_BASE="${OPENWRT_MIRROR_BASE%/}"

    sed "s#https://downloads.openwrt.org#$MIRROR_BASE#g; s#http://downloads.openwrt.org#$MIRROR_BASE#g" \
        /etc/apk/repositories.d/distfeeds.list > /tmp/distfeeds.list.$$ && \
        mv /tmp/distfeeds.list.$$ /etc/apk/repositories.d/distfeeds.list
    log "使用 OpenWrt 镜像源: $MIRROR_BASE"
}

apk_update_safe() {
    if command -v timeout >/dev/null 2>&1; then
        timeout 120 apk update
    else
        apk update
    fi
}

apk_add_safe() {
    LIMIT="$1"
    shift

    if command -v timeout >/dev/null 2>&1; then
        timeout "$LIMIT" apk add --force-broken-world "$@"
    else
        apk add --force-broken-world "$@"
    fi
}

apk_add_retry() {
    LIMIT="$1"
    shift

    TRY=1
    while [ "$TRY" -le "$APK_ADD_ATTEMPTS" ]; do
        [ "$TRY" = "1" ] || log "apk add retry $TRY/$APK_ADD_ATTEMPTS: $*"

        if apk_add_safe "$LIMIT" "$@"; then
            return 0
        fi

        apk cache clean >/dev/null 2>&1 || true
        TRY=$((TRY + 1))
        [ "$TRY" -le "$APK_ADD_ATTEMPTS" ] && sleep "$APK_ADD_RETRY_SLEEP"
    done

    return 1
}

repo_has_pkg() {
    if [ -s "$APK_INDEX" ]; then
        awk -v p="$1" '
            BEGIN { prefix = p "-" }
            index($0, prefix) == 1 { found = 1 }
            END { exit found ? 0 : 1 }
        ' "$APK_INDEX"
        return $?
    fi

    apk list -a "$1" 2>/dev/null | awk -v p="$1" '
        BEGIN { prefix = p "-" }
        index($0, prefix) == 1 { found = 1 }
        END { exit found ? 0 : 1 }
    '
}

policy_has_tag() {
    NAME="$1"
    TAG="$2"
    apk policy "$NAME" 2>/dev/null | grep -q "$TAG "
}

is_myfeed_pkg() {
    echo "$1" | grep -Eq "$MYFEED_RE"
}

emit_target_if_available() {
    NAME="$1"
    repo_has_pkg "$NAME" || return 0
    apk info -e "$NAME" >/dev/null 2>&1 && ! is_myfeed_pkg "$NAME" && return 0
    resolve_install_name "$NAME"
}

emit_first_available_target() {
    for NAME in "$@"; do
        repo_has_pkg "$NAME" || continue
        resolve_install_name "$NAME"
        return $?
    done
    return 0
}

resolve_install_name() {
    NAME="${1%@*}"

    if is_myfeed_pkg "$NAME"; then
        if policy_has_tag "$NAME" "@myfeed"; then
            echo "${NAME}@myfeed"
            return 0
        fi

        log "NOT_IN_MYFEED: $NAME"
        return 1
    fi

    echo "$NAME"
}

add_related_targets() {
    case "$1" in
        luci-app-lucky)       set -- lucky ;;
        luci-app-easytier)    set -- easytier ;;
        luci-app-rtp2httpd)   set -- rtp2httpd ;;
        luci-app-fakehttp)    set -- fakehttp ;;
        luci-app-smartdns|luci-app-smartdns-lite)
                               set -- smartdns ;;
        luci-app-bandix)      set -- bandix ;;
        luci-app-nikki)       set -- nikki ;;
        luci-app-dockerman)
                               emit_target_if_available docker || return 1
                               emit_target_if_available dockerd || return 1
                               emit_target_if_available containerd || return 1
                               emit_target_if_available runc || return 1
                               emit_first_available_target docker-compose-v2 docker-compose || return 1
                               return 0
                               ;;
        luci-app-vlmcsd)      set -- vlmcsd ;;
        luci-app-watchcat)    set -- watchcat ;;
        luci-app-oaf)         set -- appfilter ;;
        luci-app-omcproxy)    set -- omcproxy ;;
        luci-app-samba4)
                               emit_target_if_available samba4-server || return 1
                               emit_first_available_target wsdd2 || return 1
                               return 0
                               ;;
        luci-app-webdav)      set -- webdav ;;
        luci-app-firewall)    set -- firewall4 ;;
        luci-app-upnp)
                               emit_first_available_target miniupnpd-nftables miniupnpd || return 1
                               return 0
                               ;;
        luci-app-netspeedtest|luci-app-speedtest)
                               emit_first_available_target speedtest-go speedtest-cli || return 1
                               return 0
                               ;;
        luci-app-diskman)
                               emit_target_if_available block-mount || return 1
                               return 0
                               ;;
        luci-app-ota)         set -- otahelper ;;
        luci-app-aurora-config)
                               emit_target_if_available luci-theme-aurora || return 1
                               return 0
                               ;;
        luci-app-quickfile)   set -- quickfile ;;
        luci-app-ramfree)     set -- ramfree ;;
        luci-app-ttyd)        set -- ttyd ;;
        *)                    return 0 ;;
    esac

    for NAME in "$@"; do
        resolve_install_name "$NAME" || return 1
    done
}

remove_from_world() {
    PKG="$1"
    [ -n "$PKG" ] || return 0
    [ -f /etc/apk/world ] || return 0
    grep -vxF "$PKG" /etc/apk/world > /tmp/world.$$ 2>/dev/null || true
    mv /tmp/world.$$ /etc/apk/world
}

target_installed() {
    BASE="${1%@*}"
    apk info -e "$BASE" >/dev/null 2>&1
}

is_luci_target() {
    BASE="${1%@*}"
    echo "$BASE" | grep -Eq '^luci-(app|i18n)-'
}

retry_missing_targets() {
    TARGET_LIST="$1"
    [ -s "$TARGET_LIST" ] || return 0

    log "批量安装失败，逐个重试未安装的软件包..."
    while read TARGET; do
        [ -n "$TARGET" ] || continue
        target_installed "$TARGET" && continue

        log "RETRY: $TARGET"
        apk_add_retry "$APK_ADD_RETRY_TIMEOUT" "$TARGET" >>"$LOG" 2>&1 || \
            log "RETRY FAILED: $TARGET"
    done < "$TARGET_LIST"
}

install_target_group() {
    TARGET_LIST="$1"
    LABEL="$2"
    LIMIT="$3"
    RETRY="$4"

    [ -s "$TARGET_LIST" ] || return 0

    N="$(wc -l < "$TARGET_LIST")"
    log "批量安装 $N 个${LABEL}..."
    if ! apk_add_retry "$LIMIT" $(cat "$TARGET_LIST") >>"$LOG" 2>&1; then
        log "WARNING: ${LABEL}批量 apk add 返回非零，详情见 $LOG"
        if [ "$RETRY" = "1" ]; then
            retry_missing_targets "$TARGET_LIST"
        else
            log "跳过 ${LABEL}逐个重试，避免单个 LuCI 页面包长时间卡住"
        fi
    fi

    while read TARGET; do
        [ -n "$TARGET" ] || continue
        BASE="${TARGET%@*}"
        target_installed "$TARGET" || {
            log "FAILED: $TARGET"
            echo "$BASE" >> "$FAILED"
            remove_from_world "$BASE"
            remove_from_world "$TARGET"
        }
    done < "$TARGET_LIST"
}

install_target_list() {
    TARGET_LIST="$1"
    LABEL="$2"
    LIMIT="$3"
    RETRY="$4"
    CHUNK_SIZE="${5:-0}"

    [ -s "$TARGET_LIST" ] || return 0

    if [ "$CHUNK_SIZE" -le 0 ] 2>/dev/null; then
        install_target_group "$TARGET_LIST" "$LABEL" "$LIMIT" "$RETRY"
        return 0
    fi

    TOTAL="$(wc -l < "$TARGET_LIST")"
    log "分批安装 $TOTAL 个${LABEL}（每批最多 $CHUNK_SIZE 个）..."

    BATCH_FILE="/tmp/restore-install-batch.$$.txt"
    : > "$BATCH_FILE"
    BATCH_COUNT=0
    BATCH_NO=1

    while read TARGET; do
        [ -n "$TARGET" ] || continue
        echo "$TARGET" >> "$BATCH_FILE"
        BATCH_COUNT=$((BATCH_COUNT + 1))

        if [ "$BATCH_COUNT" -ge "$CHUNK_SIZE" ]; then
            install_target_group "$BATCH_FILE" "${LABEL} #$BATCH_NO" "$LIMIT" "$RETRY"
            : > "$BATCH_FILE"
            BATCH_COUNT=0
            BATCH_NO=$((BATCH_NO + 1))
        fi
    done < "$TARGET_LIST"

    if [ -s "$BATCH_FILE" ]; then
        install_target_group "$BATCH_FILE" "${LABEL} #$BATCH_NO" "$LIMIT" "$RETRY"
    fi

    rm -f "$BATCH_FILE"
}

drop_unwanted_luci_pages() {
    DROP_LIST=/tmp/restore-drop-luci-list.txt
    apk info 2>/dev/null | grep -E "$DROP_LUCI_RE" | sort -u > "$DROP_LIST" || true

    [ -s "$DROP_LIST" ] || return 0

    N="$(wc -l < "$DROP_LIST")"
    log "批量移除 $N 个不需要的 LuCI 页面..."
    cat "$DROP_LIST" | tee -a "$LOG"

    apk del --force-broken-world $(cat "$DROP_LIST") >>"$LOG" 2>&1 || \
        log "WARNING: batch luci page removal returned non-zero"

    while read PKG; do
        [ -n "$PKG" ] || continue
        remove_from_world "$PKG"
    done < "$DROP_LIST"
}

repair_luci_runtime() {
    FIX_LIST=""

    for PKG in \
        luci-base \
        luci-mod-status \
        luci-mod-network \
        luci-mod-system \
        luci-nginx \
        rpcd-mod-luci \
        rpcd-mod-ucode \
        rpcd-mod-file \
        rpcd-mod-iwinfo \
        nginx-mod-luci \
        nginx-mod-ubus \
        uwsgi-luci-support \
        luci-theme-aurora \
        luci-theme-bootstrap
    do
        apk info -e "$PKG" >/dev/null 2>&1 || continue
        FIX_LIST="$FIX_LIST $PKG"
    done

    if [ -n "$FIX_LIST" ]; then
        log "修复 LuCI 核心运行时..."
        apk fix $FIX_LIST >>"$LOG" 2>&1 || \
            log "WARNING: luci runtime fix returned non-zero"
    fi

    if apk info -e luci-theme-aurora >/dev/null 2>&1; then
        uci set luci.main.mediaurlbase='/luci-static/aurora' 2>/dev/null || true
        uci commit luci 2>/dev/null || true
    elif apk info -e luci-theme-bootstrap >/dev/null 2>&1; then
        uci set luci.main.mediaurlbase='/luci-static/bootstrap' 2>/dev/null || true
        uci commit luci 2>/dev/null || true
    fi
}

list_has_pkg_pattern() {
    PATTERN="$1"
    [ -s "$LIST" ] || return 1
    grep -Eq "$PATTERN" "$LIST"
}

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

    if [ "$DISTRIB_ARCH" != "x86_64" ]; then
        log "SKIP myfeed: unsupported arch $DISTRIB_ARCH"
        return 0
    fi

    MY_BASE="https://openwrt-packages.pages.dev"
    MY_REPO="$MY_BASE/openwrt-25.12/$DISTRIB_ARCH/myfeed/packages.adb"

    mkdir -p /etc/apk/keys /etc/apk/repositories.d

    wget -O /etc/apk/keys/myfeed.pem "$MY_BASE/public-key.pem" >>"$LOG" 2>&1 || {
        log "WARNING: myfeed key download failed"
        rm -f /etc/apk/keys/myfeed.pem
        return 0
    }

    echo "@myfeed $MY_REPO" > /etc/apk/repositories.d/00-myfeed.list
    log "Added myfeed (tagged @myfeed): $MY_REPO"
}

disable_repo_file() {
    FILE="$1"
    [ -f "$FILE" ] || return 0
    mv "$FILE" "$FILE.disabled"
    log "Disabled repo file: $FILE"
}

install_fakehttp_kmods_if_needed() {
    list_has_pkg_pattern '^(fakehttp|luci-app-fakehttp|luci-i18n-fakehttp-)' || [ -x /etc/init.d/fakehttp ] || return 0

    KMODS=""
    for PKG in kmod-nfnetlink-queue kmod-nft-queue; do
        apk info -e "$PKG" >/dev/null 2>&1 || KMODS="$KMODS $PKG"
    done

    [ -n "$KMODS" ] || {
        log "FakeHTTP NFQUEUE 模块已存在，跳过补装"
        return 0
    }

    log "补装 FakeHTTP 必需 NFQUEUE 模块..."
    apk_add_retry "$APK_ADD_RETRY_TIMEOUT" $KMODS >>"$LOG" 2>&1 || {
        log "WARNING: FakeHTTP kmod install failed"
        for PKG in $KMODS; do
            echo "$PKG" >> "$FAILED"
            remove_from_world "$PKG"
        done
    }
}

install_oaf_kmods_if_needed() {
    list_has_pkg_pattern '^(appfilter|luci-app-oaf|luci-i18n-oaf-)' || [ -x /etc/init.d/appfilter ] || return 0

    apk info -e kmod-oaf >/dev/null 2>&1 && {
        log "应用过滤 OAF 模块已存在，跳过补装"
        return 0
    }

    log "补装应用过滤必需 OAF 模块..."
    apk_add_retry "$APK_ADD_RETRY_TIMEOUT" kmod-oaf >>"$LOG" 2>&1 || {
        log "WARNING: OAF kmod install failed"
        echo "kmod-oaf" >> "$FAILED"
        remove_from_world "kmod-oaf"
    }
}

: > "$LOG"
: > "$FAILED"
: > "$NOT_IN_REPO"

log "== post restore reinstall start =="
log "软件恢复白名单：截图保留的 LuCI 功能项"

stash_app_configs

quiesce_network_addons

if [ -s /root/restore-meta/world.skipped-by-allowlist ]; then
    log "以下旧 world 包不在白名单，已跳过："
    cat /root/restore-meta/world.skipped-by-allowlist | tee -a "$LOG"
fi

if [ -s /root/restore-meta/packages.pruned-by-allowlist ]; then
    log "以下旧 overlay 包文件已按白名单清理："
    cat /root/restore-meta/packages.pruned-by-allowlist | tee -a "$LOG"
fi

log "恢复当前固件 apk world 和源..."
[ -f /rom/etc/apk/world ] && cp /rom/etc/apk/world /etc/apk/world

drop_unwanted_luci_pages

restore_distfeeds

[ -s /root/restore-meta/customfeeds.list ] && \
    cp /root/restore-meta/customfeeds.list /etc/apk/repositories.d/customfeeds.list

log "添加当前固件匹配的自用 feed 和内核模块源..."
add_myfeed

KERNEL_ID="$(sed -n 's/^kernel=//p' /rom/etc/apk/world)"
if [ -n "$KERNEL_ID" ]; then
    echo "https://core3.cooluc.com/x86_64/${KERNEL_ID}/packages.adb" > /etc/apk/repositories.d/core3.list
fi

UPDATE_OK=0

log "apk update..."
if apk_update_safe >>"$LOG" 2>&1; then
    UPDATE_OK=1
else
    log "WARNING: apk update 失败，临时禁用旧 customfeeds 后重试"
    disable_repo_file /etc/apk/repositories.d/customfeeds.list

    if apk_update_safe >>"$LOG" 2>&1; then
        UPDATE_OK=1
    else
        log "WARNING: apk update 仍失败，临时禁用 myfeed 后重试"
        disable_repo_file /etc/apk/repositories.d/00-myfeed.list

        if apk_update_safe >>"$LOG" 2>&1; then
            UPDATE_OK=1
        else
            log "ERROR: apk update 仍失败，跳过普通软件包恢复，避免污染 /etc/apk/world"
        fi
    fi
fi

if [ "$UPDATE_OK" = "1" ]; then
    apk list -a > "$APK_INDEX" 2>/dev/null || : > "$APK_INDEX"
    install_fakehttp_kmods_if_needed
    install_oaf_kmods_if_needed
fi

INSTALL_LIST=/tmp/restore-install-list.txt
INSTALL_CORE_LIST=/tmp/restore-install-core-list.txt
INSTALL_LUCI_LIST=/tmp/restore-install-luci-list.txt
: > "$INSTALL_LIST"
: > "$INSTALL_CORE_LIST"
: > "$INSTALL_LUCI_LIST"

if [ "$UPDATE_OK" = "1" ] && [ -s "$LIST" ]; then
    TOTAL="$(wc -l < "$LIST")"
    log "扫描旧清单 $TOTAL 个包，构建批量安装目标..."

    while read PKG; do
        [ -n "$PKG" ] || continue
        BASE="${PKG%@*}"

        echo "$BASE" | grep -Eq "$ALLOW_RE" || {
            log "非白名单包，不恢复: $BASE"
            remove_from_world "$BASE"
            remove_from_world "$PKG"
            continue
        }

        if apk info -e "$BASE" >/dev/null 2>&1 && ! is_myfeed_pkg "$BASE"; then
            RELATED_TARGETS="$(add_related_targets "$BASE")" || {
                echo "$BASE" >> "$NOT_IN_REPO"
                remove_from_world "$BASE"
                remove_from_world "$PKG"
                continue
            }

            [ -n "$RELATED_TARGETS" ] && printf '%s\n' "$RELATED_TARGETS" | sed '/^$/d' >> "$INSTALL_LIST"
            continue
        fi

        if ! repo_has_pkg "$BASE"; then
            echo "$BASE" >> "$NOT_IN_REPO"
            remove_from_world "$BASE"
            remove_from_world "$PKG"
            continue
        fi

        TARGETS="$(resolve_install_name "$BASE")" || {
            echo "$BASE" >> "$NOT_IN_REPO"
            remove_from_world "$BASE"
            remove_from_world "$PKG"
            continue
        }

        RELATED_TARGETS="$(add_related_targets "$BASE")" || {
            echo "$BASE" >> "$NOT_IN_REPO"
            remove_from_world "$BASE"
            remove_from_world "$PKG"
            continue
        }

        printf '%s\n' "$TARGETS" "$RELATED_TARGETS" | sed '/^$/d' >> "$INSTALL_LIST"
    done < "$LIST"

    if [ -s "$INSTALL_LIST" ]; then
        sort -u "$INSTALL_LIST" > "$INSTALL_LIST.sorted"
        mv "$INSTALL_LIST.sorted" "$INSTALL_LIST"

        while read TARGET; do
            [ -n "$TARGET" ] || continue
            if is_luci_target "$TARGET"; then
                echo "$TARGET" >> "$INSTALL_LUCI_LIST"
            else
                echo "$TARGET" >> "$INSTALL_CORE_LIST"
            fi
        done < "$INSTALL_LIST"

        install_target_list "$INSTALL_CORE_LIST" "核心/后端包" "$APK_ADD_BATCH_TIMEOUT" "1" "$APK_ADD_CORE_CHUNK_SIZE"
        install_target_list "$INSTALL_LUCI_LIST" "LuCI 页面包" "$APK_ADD_LUCI_TIMEOUT" "$APK_RETRY_LUCI"
    else
        log "没有需要新装的包"
    fi
else
    [ -s "$LIST" ] || log "没有找到软件包恢复清单：$LIST"
fi

drop_unwanted_luci_pages
repair_luci_runtime
restore_app_configs

log "复查失败列表，移除实际已经安装成功的软件包..."
TMP_FAILED=/root/apk-install-failed.tmp
: > "$TMP_FAILED"

while read PKG; do
    [ -n "$PKG" ] || continue
    apk info -e "$PKG" >/dev/null 2>&1 || echo "$PKG" >> "$TMP_FAILED"
done < "$FAILED"

mv "$TMP_FAILED" "$FAILED"

log "清理 world 中源里没有或安装失败的软件包..."
cat "$FAILED" "$NOT_IN_REPO" 2>/dev/null | sort -u | while read PKG; do
    [ -n "$PKG" ] || continue
    remove_from_world "$PKG"
done

log "重启服务..."
rm -rf /tmp/luci-indexcache /tmp/luci-modulecache /tmp/luci-*cache 2>/dev/null || true
/etc/init.d/rpcd restart 2>/dev/null || true
/etc/init.d/uwsgi restart 2>/dev/null || true
/etc/init.d/nginx restart 2>/dev/null || true
/etc/init.d/uhttpd restart 2>/dev/null || true

restore_network_addons

if [ -x /etc/init.d/iptv-refresh-httpd ]; then
    /etc/init.d/iptv-refresh-httpd enable 2>/dev/null || true
    /etc/init.d/iptv-refresh-httpd restart 2>/dev/null || \
        /etc/init.d/iptv-refresh-httpd start 2>/dev/null || true
fi

log "== done =="
log "安装失败的软件包：$FAILED"
cat "$FAILED" | tee -a "$LOG"

log "源里不存在的软件包：$NOT_IN_REPO"
cat "$NOT_IN_REPO" | tee -a "$LOG"
POSTEOF

chmod +x "$UP/root/post_restore_reinstall.sh"

echo "第一阶段完成，系统即将重启。"
echo "重启后 SSH 执行："
echo "/root/post_restore_reinstall.sh"

sync
reboot
