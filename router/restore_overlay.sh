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

echo "校验备份包..."
gzip -t "$BACKUP_FILE"
tar -tzf "$BACKUP_FILE" >/dev/null

echo "清空并恢复 overlay..."
rm -rf /overlay/*
tar -xzf "$BACKUP_FILE" -C /

UP=/overlay/upper
mkdir -p "$UP/root/restore-meta"

ALLOW_RE='^(luci-app-(smartdns|smartdns-lite|dockerman|lucky|vlmcsd|fakehttp|watchcat|oaf|nikki|omcproxy|rtp2httpd|samba4|webdav|easytier|bandix|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|diskman|ttyd|commands|filebrowser|filemanager|fileassistant|autoreboot|timedreboot|aurora-config)|luci-i18n-(smartdns|smartdns-lite|fakehttp|lucky|easytier|rtp2httpd|bandix|nikki|vlmcsd|watchcat|oaf|samba4|webdav|omcproxy|dockerman|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|diskman|ttyd|commands|filebrowser|filemanager|fileassistant|autoreboot|timedreboot|aurora-config)-zh-cn|luci-i18n-app-omcproxy-zh-cn|luci-theme-aurora|python3|python3-requests|tcpdump|curl|bash)$'
SKIPPED_BY_ALLOW="$UP/root/restore-meta/world.skipped-by-allowlist"
PRUNED_BY_ALLOW="$UP/root/restore-meta/packages.pruned-by-allowlist"
CURRENT_WORLD="$UP/root/restore-meta/world.from-current-rom"
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
rm -rf "$UP/etc/smartdns/data"

rm -rf /overlay/work /overlay/lost+found

cat > "$UP/root/post_restore_reinstall.sh" <<'POSTEOF'
#!/bin/sh

LOG=/root/post_restore_reinstall.log
FAILED=/root/apk-install-failed.txt
NOT_IN_REPO=/root/apk-not-in-repo.txt
LIST=/root/apk-world.restore-list
ALLOW_RE='^(luci-app-(smartdns|smartdns-lite|dockerman|lucky|vlmcsd|fakehttp|watchcat|oaf|nikki|omcproxy|rtp2httpd|samba4|webdav|easytier|bandix|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|diskman|ttyd|commands|filebrowser|filemanager|fileassistant|autoreboot|timedreboot|aurora-config)|luci-i18n-(smartdns|smartdns-lite|fakehttp|lucky|easytier|rtp2httpd|bandix|nikki|vlmcsd|watchcat|oaf|samba4|webdav|omcproxy|dockerman|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|diskman|ttyd|commands|filebrowser|filemanager|fileassistant|autoreboot|timedreboot|aurora-config)-zh-cn|luci-i18n-app-omcproxy-zh-cn|luci-theme-aurora|python3|python3-requests|tcpdump|curl|bash)$'
DROP_LUCI_RE='^luci-(app|i18n)-(wolplus|zerotier|sqm|socat|qbittorrent|passwall|passwall2|openlist|openlist2|natmap|nlbwmon|mosdns|homeproxy|frpc|eqos|argon-config|airplay2|airconnect|usb-printer|mentohust|ddns|ssr-plus|openclash)(-|$)|^luci-proto-wireguard$|^luci-i18n-proto-wireguard-'
MYFEED_RE='^(lucky|luci-app-lucky|luci-i18n-lucky-zh-cn|easytier|luci-app-easytier|luci-i18n-easytier-zh-cn|rtp2httpd|luci-app-rtp2httpd|luci-i18n-rtp2httpd-zh-cn|fakehttp|luci-app-fakehttp|luci-i18n-fakehttp-zh-cn|smartdns|luci-app-smartdns|luci-app-smartdns-lite|bandix|luci-app-bandix|luci-i18n-bandix-zh-cn|nikki|luci-app-nikki|luci-i18n-nikki-ru|luci-i18n-nikki-zh-cn|luci-i18n-nikki-zh-tw)$'

log() {
    echo "$@" | tee -a "$LOG"
}

apk_update_safe() {
    if command -v timeout >/dev/null 2>&1; then
        timeout 120 apk update
    else
        apk update
    fi
}

repo_has_pkg() {
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

    if policy_has_tag "$NAME" "@nikki"; then
        echo "${NAME}@nikki"
    else
        echo "$NAME"
    fi
}

add_myfeed_related_targets() {
    case "$1" in
        luci-app-lucky)       set -- lucky ;;
        luci-app-easytier)    set -- easytier ;;
        luci-app-rtp2httpd)   set -- rtp2httpd ;;
        luci-app-fakehttp)    set -- fakehttp ;;
        luci-app-smartdns|luci-app-smartdns-lite)
                               set -- smartdns ;;
        luci-app-bandix)      set -- bandix ;;
        luci-app-nikki)       set -- nikki ;;
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

drop_unwanted_luci_pages() {
    apk info 2>/dev/null | grep -E "$DROP_LUCI_RE" | while read PKG; do
        [ -n "$PKG" ] || continue
        log "强制移除不需要的 LuCI 页面: $PKG"
        apk del --force-broken-world "$PKG" >>"$LOG" 2>&1 || \
            log "WARNING: failed to remove $PKG"
        remove_from_world "$PKG"
    done
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

add_nikki_feed_if_needed() {
    list_has_pkg_pattern '^(nikki|luci-app-nikki|luci-i18n-nikki-|mihomo|mihomo-meta|mihomo-alpha)' || return 0

    [ -f /etc/openwrt_release ] || return 0
    . /etc/openwrt_release

    case "$DISTRIB_RELEASE" in
        *"24.10"*) NIKKI_BRANCH="openwrt-24.10" ;;
        *"25.12"*) NIKKI_BRANCH="openwrt-25.12" ;;
        "SNAPSHOT") NIKKI_BRANCH="SNAPSHOT" ;;
        *)
            log "SKIP Nikki feed: unsupported release $DISTRIB_RELEASE"
            return 0
            ;;
    esac

    NIKKI_BASE="https://nikkinikki.pages.dev"
    NIKKI_REPO="$NIKKI_BASE/$NIKKI_BRANCH/$DISTRIB_ARCH/nikki/packages.adb"

    mkdir -p /etc/apk/keys /etc/apk/repositories.d

    wget -O /etc/apk/keys/nikki.pem "$NIKKI_BASE/public-key.pem" >>"$LOG" 2>&1 || {
        log "WARNING: Nikki key download failed"
        rm -f /etc/apk/keys/nikki.pem
        return 0
    }

    echo "@nikki $NIKKI_REPO" > /etc/apk/repositories.d/20-nikki.list
    log "Added Nikki feed (tagged @nikki): $NIKKI_REPO"
}

disable_repo_file() {
    FILE="$1"
    [ -f "$FILE" ] || return 0
    mv "$FILE" "$FILE.disabled"
    log "Disabled repo file: $FILE"
}

install_fakehttp_kmods_if_needed() {
    list_has_pkg_pattern '^(fakehttp|luci-app-fakehttp|luci-i18n-fakehttp-)' || [ -x /etc/init.d/fakehttp ] || return 0

    log "补装 FakeHTTP 必需 NFQUEUE 模块..."
    apk add --force-broken-world kmod-nfnetlink-queue kmod-nft-queue >>"$LOG" 2>&1 || {
        log "WARNING: FakeHTTP kmod install failed"
        echo "kmod-nfnetlink-queue" >> "$FAILED"
        echo "kmod-nft-queue" >> "$FAILED"
        remove_from_world "kmod-nfnetlink-queue"
        remove_from_world "kmod-nft-queue"
    }
}

install_oaf_kmods_if_needed() {
    list_has_pkg_pattern '^(appfilter|luci-app-oaf|luci-i18n-oaf-)' || [ -x /etc/init.d/appfilter ] || return 0

    log "补装应用过滤必需 OAF 模块..."
    apk add --force-broken-world kmod-oaf >>"$LOG" 2>&1 || {
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

if [ -s /root/restore-meta/world.skipped-by-allowlist ]; then
    log "以下旧 world 包不在白名单，已跳过："
    cat /root/restore-meta/world.skipped-by-allowlist | tee -a "$LOG"
fi

if [ -s /root/restore-meta/packages.pruned-by-allowlist ]; then
    log "以下旧 overlay 包文件已按白名单清理："
    cat /root/restore-meta/packages.pruned-by-allowlist | tee -a "$LOG"
fi

[ -x /etc/init.d/fakehttp ] && /etc/init.d/fakehttp stop 2>/dev/null || true

log "恢复当前固件 apk world 和源..."
[ -f /rom/etc/apk/world ] && cp /rom/etc/apk/world /etc/apk/world

drop_unwanted_luci_pages

mkdir -p /etc/apk/repositories.d

[ -f /rom/etc/apk/repositories.d/distfeeds.list ] && \
    cp /rom/etc/apk/repositories.d/distfeeds.list /etc/apk/repositories.d/distfeeds.list

[ -s /root/restore-meta/customfeeds.list ] && \
    cp /root/restore-meta/customfeeds.list /etc/apk/repositories.d/customfeeds.list

log "添加当前固件匹配的自用 feed 和内核模块源..."
add_myfeed
add_nikki_feed_if_needed

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
        log "WARNING: apk update 仍失败，临时禁用 myfeed/Nikki 后重试"
        disable_repo_file /etc/apk/repositories.d/00-myfeed.list
        disable_repo_file /etc/apk/repositories.d/20-nikki.list

        if apk_update_safe >>"$LOG" 2>&1; then
            UPDATE_OK=1
        else
            log "ERROR: apk update 仍失败，跳过普通软件包恢复，避免污染 /etc/apk/world"
        fi
    fi
fi

if [ "$UPDATE_OK" = "1" ]; then
    install_fakehttp_kmods_if_needed
    install_oaf_kmods_if_needed
fi

INSTALL_LIST=/tmp/restore-install-list.txt
ENFORCE_LIST=/tmp/restore-enforce-list.txt
: > "$INSTALL_LIST"
: > "$ENFORCE_LIST"

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

        apk info -e "$BASE" >/dev/null 2>&1 && continue

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

        RELATED_TARGETS="$(add_myfeed_related_targets "$BASE")" || {
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

        N="$(wc -l < "$INSTALL_LIST")"
        log "批量安装 $N 个包（一次 apk add 解依赖）..."
        if ! apk add --force-broken-world $(cat "$INSTALL_LIST") >>"$LOG" 2>&1; then
            log "WARNING: 批量 apk add 返回非零，可能部分包失败，详情见 $LOG"
        fi
        while read TARGET; do
            BASE="${TARGET%@*}"
            apk info -e "$BASE" >/dev/null 2>&1 || {
                log "FAILED: $TARGET"
                echo "$BASE" >> "$FAILED"
                remove_from_world "$BASE"
                remove_from_world "$TARGET"
            }
        done < "$INSTALL_LIST"
    else
        log "没有需要新装的包"
    fi
else
    [ -s "$LIST" ] || log "没有找到软件包恢复清单：$LIST"
fi

if [ "$UPDATE_OK" = "1" ]; then
    log "扫描已装包，构建 @myfeed / @nikki 强制覆盖清单..."
    apk info 2>/dev/null | sort -u | while read PKG; do
        [ -n "$PKG" ] || continue
        case "$PKG" in
            kmod-*) continue ;;
        esac
        POLICY="$(apk policy "$PKG" 2>/dev/null)"
        case "$POLICY" in
            *"@myfeed "*) echo "${PKG}@myfeed" >> "$ENFORCE_LIST" ;;
            *"@nikki "*)  echo "${PKG}@nikki"  >> "$ENFORCE_LIST" ;;
        esac
    done

    if [ -s "$ENFORCE_LIST" ]; then
        N="$(wc -l < "$ENFORCE_LIST")"
        log "强制覆盖 $N 个包到 @tag 版本（一次 apk add）..."
        apk add --force-broken-world $(cat "$ENFORCE_LIST") >>"$LOG" 2>&1 || \
            log "WARNING: 批量 enforce 返回非零，详情见 $LOG"
    else
        log "没有需要强制覆盖的包"
    fi
fi

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
/etc/init.d/uhttpd restart 2>/dev/null || true

if [ -x /etc/init.d/smartdns ]; then
    /etc/init.d/smartdns restart 2>/dev/null || true
fi

if [ -x /etc/init.d/fakehttp ]; then
    /etc/init.d/fakehttp enable 2>/dev/null || true
    /etc/init.d/fakehttp restart 2>/dev/null || true
fi

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
