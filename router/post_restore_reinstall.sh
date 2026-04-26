#!/bin/sh

LOG=/root/post_restore_reinstall.log
FAILED=/root/apk-install-failed.txt
NOT_IN_REPO=/root/apk-not-in-repo.txt
LIST=/root/apk-world.restore-list
APK_INDEX=/tmp/restore-apk-list.txt
APP_CONFIG_STASH=/root/restore-meta/app-config-stash.tar.gz
SERVICE_STATE=/root/restore-meta/network-addon-services.state
LUCI_COMMANDS_STASH=/root/restore-meta/luci.commands.from-backup
APK_ADD_BATCH_TIMEOUT="${APK_ADD_BATCH_TIMEOUT:-300}"
APK_ADD_LUCI_TIMEOUT="${APK_ADD_LUCI_TIMEOUT:-300}"
APK_ADD_RETRY_TIMEOUT="${APK_ADD_RETRY_TIMEOUT:-120}"
APK_ADD_ATTEMPTS="${APK_ADD_ATTEMPTS:-3}"
APK_ADD_RETRY_SLEEP="${APK_ADD_RETRY_SLEEP:-5}"
APK_ADD_CORE_CHUNK_SIZE="${APK_ADD_CORE_CHUNK_SIZE:-6}"
APK_RETRY_LUCI="${APK_RETRY_LUCI:-0}"
NETWORK_FAIL_THRESHOLD="${NETWORK_FAIL_THRESHOLD:-5}"
NETWORK_CLASSIFIED=0
NETWORK_DEAD=0
NETWORK_DIAG_DONE=0
MYFEED_DEAD=0
MYFEED_DIAG_DONE=0
RESTORE_PROXY_UP="${RESTORE_PROXY_UP:-1}"
KEEP_PROXY_UP="${KEEP_PROXY_UP:-$RESTORE_PROXY_UP}"
RESTORE_LUCI_WRAPPER="${RESTORE_LUCI_WRAPPER:-1}"
MYFEED_LUCI_COMPAT="${MYFEED_LUCI_COMPAT:-1}"
RESTORE_KEEP_BACKUP_PACKAGES="${RESTORE_KEEP_BACKUP_PACKAGES:-smartdns lucky nikki luci-app-smartdns luci-app-smartdns-lite luci-app-lucky luci-i18n-lucky-zh-cn luci-app-nikki luci-i18n-nikki-zh-cn luci-i18n-nikki-zh-tw luci-i18n-nikki-ru}"
RESTORE_KEEP_MYFEED_WHEN_FEED_DOWN="${RESTORE_KEEP_MYFEED_WHEN_FEED_DOWN:-1}"
MYFEED_BASE="${MYFEED_BASE:-}"
MYFEED_BASES="${MYFEED_BASES:-${MYFEED_BASE:+$MYFEED_BASE }https://openwrt-packages.pages.dev}"
MYFEED_FORCE_IPV4="${MYFEED_FORCE_IPV4:-0}"
MYFEED_IPV4="${MYFEED_IPV4:-}"
OPENWRT_MIRROR_BASE="${OPENWRT_MIRROR_BASE:-https://mirrors.cloud.tencent.com/openwrt}"
COOLUC_PACKAGES_FEED_BASE="${COOLUC_PACKAGES_FEED_BASE:-https://opkg.cooluc.com}"
ALLOW_RE='^(luci-app-(smartdns|smartdns-lite|dockerman|lucky|vlmcsd|fakehttp|watchcat|oaf|nikki|omcproxy|rtp2httpd|samba4|webdav|easytier|bandix|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|ota|diskman|ttyd|commands|quickfile|filebrowser|filemanager|fileassistant|ramfree|autoreboot|timedreboot|aurora-config)|luci-i18n-(smartdns|smartdns-lite|fakehttp|lucky|easytier|rtp2httpd|bandix|nikki|vlmcsd|watchcat|oaf|samba4|webdav|omcproxy|dockerman|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|ota|diskman|ttyd|commands|quickfile|filebrowser|filemanager|fileassistant|ramfree|autoreboot|timedreboot|aurora-config)-zh-cn|luci-i18n-app-omcproxy-zh-cn|luci-theme-aurora|lucky|easytier|rtp2httpd|fakehttp|smartdns|bandix|nikki|yq|mihomo|sing-box|ip-full|firewall4|ca-bundle|ca-certificates|python3|python3-requests|tcpdump|curl|bash)$'
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
        etc/config/dockerd etc/docker \
        etc/config/samba4 \
        etc/config/ttyd etc/config/lucky \
        etc/config/luci-app-commands etc/config/commands \
        etc/config/aurora www/luci-static/aurora/images \
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

restore_luci_commands() {
    [ -s "$LUCI_COMMANDS_STASH" ] || return 0

    log "还原 LuCI 自定义命令配置..."
    mkdir -p /etc/config /root/restore-meta
    [ -f /etc/config/luci ] || : > /etc/config/luci
    cp /etc/config/luci /root/restore-meta/luci.before-commands-restore 2>/dev/null || true

    TMP=/tmp/restore-luci-no-commands.$$
    awk '
        /^[ \t]*config[ \t]+/ {
            skip = ($2 == "command" || $2 == "'\''command'\''" || $2 == "\"command\"")
        }
        !skip {
            print
        }
    ' /etc/config/luci > "$TMP" || {
        rm -f "$TMP"
        log "WARNING: failed to prepare luci config for command restore"
        return 0
    }

    {
        cat "$TMP"
        echo
        cat "$LUCI_COMMANDS_STASH"
    } > /etc/config/luci
    rm -f "$TMP"
}

service_enabled() {
    [ -x "/etc/init.d/$1" ] && /etc/init.d/"$1" enabled >/dev/null 2>&1
}

quiesce_network_addons() {
    mkdir -p /root/restore-meta
    : > "$SERVICE_STATE"

    KEEP_RUNNING=""
    if [ "$KEEP_PROXY_UP" = "1" ]; then
        KEEP_RUNNING="smartdns nikki"
        log "KEEP_PROXY_UP=1：保留 smartdns + nikki 运行（apk 操作通过代理走外网，避开 GFW 对 Cloudflare TLS 的干扰）"
    fi

    for SVC in smartdns nikki fakehttp appfilter oaf; do
        [ -x "/etc/init.d/$SVC" ] || continue

        case " $KEEP_RUNNING " in
            *" $SVC "*)
                log "保持运行（不停止）: $SVC"
                continue
                ;;
        esac

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

service_running() {
    [ -x "/etc/init.d/$1" ] && /etc/init.d/"$1" running >/dev/null 2>&1
}

proxy_backend_available() {
    SVC="$1"
    [ -x "/etc/init.d/$SVC" ] || return 1

    case "$SVC" in
        smartdns)
            command -v smartdns >/dev/null 2>&1 || [ -x /usr/sbin/smartdns ] || return 1
            ;;
        nikki)
            command -v nikki >/dev/null 2>&1 || \
                [ -x /usr/bin/nikki ] || [ -x /usr/bin/mihomo ] || return 1
            ;;
    esac

    return 0
}

start_service_for_restore_proxy() {
    SVC="$1"

    proxy_backend_available "$SVC" || {
        log "代理后端不可用，跳过拉起: $SVC"
        return 1
    }

    if service_running "$SVC"; then
        log "代理后端已运行: $SVC"
        return 0
    fi

    log "拉起代理后端: $SVC"
    /etc/init.d/"$SVC" restart >>"$LOG" 2>&1 || \
        /etc/init.d/"$SVC" start >>"$LOG" 2>&1 || true

    sleep 1
    service_running "$SVC" && {
        log "代理后端已运行: $SVC"
        return 0
    }

    log "WARNING: 代理后端启动失败: $SVC"
    return 1
}

probe_restore_proxy() {
    for URL in \
        https://openwrt-packages.pages.dev/ \
        https://mirrors.cloud.tencent.com/
    do
        timeout 15 wget -O /dev/null "$URL" >/dev/null 2>&1 && {
            log "代理/出站探测 OK: $URL"
            return 0
        }
    done

    log "WARNING: 代理/出站探测未通过，继续按源可用性恢复"
    return 1
}

bring_up_restore_proxy() {
    [ "$RESTORE_PROXY_UP" = "1" ] || {
        log "RESTORE_PROXY_UP=0：不主动拉起 smartdns/nikki"
        return 0
    }

    log "尝试拉起 smartdns + nikki，供路由器本机 apk 流量使用..."
    rm -f /etc/smartdns/smartdns.cache 2>/dev/null || true
    rm -f /etc/smartdns/data/smartdns.cache 2>/dev/null || true

    STARTED=0
    start_service_for_restore_proxy smartdns && STARTED=1
    start_service_for_restore_proxy nikki && STARTED=1

    [ "$STARTED" = "1" ] || {
        log "WARNING: smartdns/nikki 均未拉起，继续普通恢复"
        return 0
    }

    sleep 3
    probe_restore_proxy || true
}

restore_distfeeds() {
    mkdir -p /etc/apk/repositories.d

    if [ -s /root/restore-meta/distfeeds.current-before-restore ] && \
       grep -q 'mirrors.cloud.tencent.com/openwrt' /root/restore-meta/distfeeds.current-before-restore && \
       grep -q 'opkg.cooluc.com/openwrt-' /root/restore-meta/distfeeds.current-before-restore && \
       grep -q 'core3.cooluc.com' /root/restore-meta/distfeeds.current-before-restore && \
       ! grep -q 'downloads.openwrt.org' /root/restore-meta/distfeeds.current-before-restore && \
       ! grep -q '/targets/.*/packages/packages\.adb' /root/restore-meta/distfeeds.current-before-restore; then
        cp /root/restore-meta/distfeeds.current-before-restore /etc/apk/repositories.d/distfeeds.list
        log "使用恢复前干净系统的 distfeeds.list"
        return 0
    fi

    if [ -s /root/restore-meta/distfeeds.current-before-restore ]; then
        log "恢复前 distfeeds.list 未通过干净源校验，改用 ROM 模板重建"
    fi

    [ -f /rom/etc/apk/repositories.d/distfeeds.list ] || {
        log "WARNING: ROM 未提供 distfeeds，跳过 OpenWrt 官方源重建"
        return 0
    }

    DISTFEEDS_TMP=/tmp/restore-distfeeds.$$
    cp /rom/etc/apk/repositories.d/distfeeds.list "$DISTFEEDS_TMP"
    log "从 ROM 模板重建干净 OpenWrt 软件源"

    grep -Ev '/targets/[^[:space:]]*/packages/packages\.adb|^[[:space:]]*#|^[[:space:]]*$' \
        "$DISTFEEDS_TMP" > /etc/apk/repositories.d/distfeeds.list

    if [ -n "$OPENWRT_MIRROR_BASE" ] && \
       grep -Eq 'https?://downloads\.openwrt\.org' /etc/apk/repositories.d/distfeeds.list; then
        MIRROR_BASE="${OPENWRT_MIRROR_BASE%/}"
        sed "s#https://downloads.openwrt.org#$MIRROR_BASE#g; s#http://downloads.openwrt.org#$MIRROR_BASE#g" \
            /etc/apk/repositories.d/distfeeds.list > "$DISTFEEDS_TMP" && \
            mv "$DISTFEEDS_TMP" /etc/apk/repositories.d/distfeeds.list
        log "已将 OpenWrt 官方源替换为镜像源: $MIRROR_BASE"
    fi

    if [ -n "$COOLUC_PACKAGES_FEED_BASE" ] && [ -f /etc/openwrt_release ]; then
        . /etc/openwrt_release
        SERIES="$(echo "$DISTRIB_RELEASE" | sed -n 's/^\([0-9][0-9]*\.[0-9][0-9]*\).*/\1/p')"
        if [ -n "$SERIES" ] && [ -n "$DISTRIB_ARCH" ]; then
            COOLUC_REPO="${COOLUC_PACKAGES_FEED_BASE%/}/openwrt-${SERIES}/${DISTRIB_ARCH}/packages.adb"
            echo "$COOLUC_REPO" >> /etc/apk/repositories.d/distfeeds.list
            log "已添加 cooluc 软件源: $COOLUC_REPO"
        fi
    fi

    KERNEL_ID="$(sed -n 's/^kernel=//p' /rom/etc/apk/world)"
    if [ -n "$KERNEL_ID" ]; then
        CORE3_REPO="https://core3.cooluc.com/x86_64/${KERNEL_ID}/packages.adb"
        echo "$CORE3_REPO" >> /etc/apk/repositories.d/distfeeds.list
        log "已添加 core3 内核模块源: $CORE3_REPO"
    fi

    rm -f "$DISTFEEDS_TMP"
}

url_host() {
    echo "$1" | sed 's#^[^:][^:]*://##; s#/.*##; s#^\[\(.*\)\]$#\1#; s#:[0-9][0-9]*$##'
}

myfeed_host() {
    url_host "$MYFEED_BASE"
}

myfeed_repo_url() {
    BASE="${1:-$MYFEED_BASE}"
    ARCH="${DISTRIB_ARCH:-x86_64}"
    echo "${BASE%/}/openwrt-25.12/$ARCH/myfeed/packages.adb"
}

myfeed_key_url() {
    BASE="${1:-$MYFEED_BASE}"
    echo "${BASE%/}/public-key.pem"
}

resolve_ipv4() {
    HOST="$1"
    nslookup "$HOST" 2>/dev/null | awk '
        $1 == "Name:" { seen = 1; next }
        seen {
            for (i = 1; i <= NF; i++) {
                if ($i ~ /^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$/) {
                    print $i
                    exit
                }
            }
        }
    '
}

pin_host_ipv4() {
    HOST="$1"
    IP="$2"

    [ -n "$HOST" ] || return 1
    [ -n "$IP" ] || IP="$(resolve_ipv4 "$HOST")"
    [ -n "$IP" ] || return 1

    TMP=/tmp/restore-hosts.$$
    [ -f /etc/hosts ] || : > /etc/hosts
    grep -v ' # restore-myfeed-ipv4$' /etc/hosts > "$TMP" 2>/dev/null || : > "$TMP"
    {
        echo "$IP $HOST # restore-myfeed-ipv4"
        cat "$TMP"
    } > /etc/hosts
    rm -f "$TMP"

    log "myfeed 使用 IPv4: $HOST -> $IP"
}

pin_myfeed_ipv4() {
    [ "$MYFEED_FORCE_IPV4" = "1" ] || return 0

    HOST="$(myfeed_host)"
    case "$HOST" in
        *:*) return 0 ;;
    esac

    pin_host_ipv4 "$HOST" "$MYFEED_IPV4" || \
        log "WARNING: myfeed IPv4 固定失败，继续使用系统 DNS"
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
        check_network_fatigue
        if myfeed_seems_dead; then
            log "myfeed 早停，停止 apk add 内部重试"
            return 1
        fi
        if network_seems_dead; then
            log "网络早停，停止 apk add 内部重试"
            return 1
        fi
        TRY=$((TRY + 1))
        [ "$TRY" -le "$APK_ADD_ATTEMPTS" ] && sleep "$APK_ADD_RETRY_SLEEP"
    done

    return 1
}

network_seems_dead() {
    [ "$NETWORK_DEAD" = "1" ]
}

myfeed_seems_dead() {
    [ "$MYFEED_DEAD" = "1" ]
}

count_recent_wget_errors() {
    [ -s "$LOG" ] || { echo 0; return; }
    tail -n 500 "$LOG" 2>/dev/null | grep -c "wget: exited with error 4"
}

https_host_available() {
    HOST="$1"
    timeout 10 wget --spider -O /dev/null "https://${HOST}/" >/dev/null 2>&1
    EC=$?
    [ "$EC" = "0" ] || [ "$EC" = "8" ]
}

official_https_available() {
    https_host_available mirrors.cloud.tencent.com || https_host_available core3.cooluc.com
}

myfeed_index_available() {
    [ -n "$MYFEED_BASE" ] || return 1
    TMP=/tmp/restore-myfeed-probe.$$
    rm -f "$TMP"
    timeout 25 wget -O "$TMP" "$(myfeed_repo_url)" >/dev/null 2>&1
    EC=$?
    [ "$EC" = "0" ] && [ -s "$TMP" ]
    OK=$?
    rm -f "$TMP"
    return "$OK"
}

classify_network_fatigue() {
    [ "$NETWORK_CLASSIFIED" = "1" ] && return 0
    NETWORK_CLASSIFIED=1

    if official_https_available && ! myfeed_index_available; then
        MYFEED_DEAD=1
        log "检测到官方源可访问，但 myfeed 索引不可达；后续只跳过 myfeed 包"
        return 0
    fi

    NETWORK_DEAD=1
}

check_network_fatigue() {
    network_seems_dead && return 0
    myfeed_seems_dead && return 0
    COUNT="$(count_recent_wget_errors)"
    [ "$COUNT" -ge "$NETWORK_FAIL_THRESHOLD" ] && classify_network_fatigue
}

announce_myfeed_dead() {
    [ "$MYFEED_DEAD" = "1" ] || return 0
    [ "$MYFEED_DIAG_DONE" = "1" ] && return 0
    MYFEED_DIAG_DONE=1
    log "myfeed 网络失败，跳过 myfeed 包；官方源包继续恢复"
    diagnose_network
}

announce_network_dead() {
    [ "$NETWORK_DEAD" = "1" ] || return 0
    [ "$NETWORK_DIAG_DONE" = "1" ] && return 0
    log "网络持续失败（日志中 $(count_recent_wget_errors) 次 wget error 4，阈值 $NETWORK_FAIL_THRESHOLD），跳过后续 apk add 重试"
    diagnose_network
}

diagnose_network() {
    [ "$NETWORK_DIAG_DONE" = "1" ] && return 0
    NETWORK_DIAG_DONE=1

    log "===== 网络诊断（自动触发） ====="
    HOSTS=""
    for BASE in $MYFEED_BASES; do
        HOST="$(url_host "$BASE")"
        [ -n "$HOST" ] || continue
        case " $HOSTS " in
            *" $HOST "*) ;;
            *) HOSTS="$HOSTS $HOST" ;;
        esac
    done
    HOSTS="$HOSTS mirrors.cloud.tencent.com core3.cooluc.com"

    log "-- myfeed IPv4 固定 --"
    grep ' # restore-myfeed-ipv4$' /etc/hosts 2>/dev/null | tee -a "$LOG" || true

    log "-- DNS 解析 --"
    for HOST in $HOSTS; do
        log "[$HOST]"
        nslookup "$HOST" 2>&1 | head -8 | tee -a "$LOG"
    done

    log "-- HTTPS 握手测试（含 TCP+TLS） --"
    for HOST in $HOSTS; do
        OUT="$(timeout 10 wget --spider -O /dev/null "https://${HOST}/" 2>&1)"
        EC=$?
        case "$EC" in
            0|8) log "  $HOST: TLS OK (wget exit $EC)" ;;
            4)
                REASON="$(echo "$OUT" | grep -Eo '(OpenSSL: error:[A-F0-9:]+|Unable to establish SSL connection|Connection refused|timed out|Network is unreachable|Connection reset|Name or service not known)' | head -1)"
                [ -n "$REASON" ] || REASON="未知失败"
                log "  $HOST: NETWORK_FAIL — $REASON"
                ;;
            5)   log "  $HOST: SSL 证书验证失败" ;;
            124) log "  $HOST: 超时" ;;
            *)   log "  $HOST: wget exit $EC" ;;
        esac
    done

    log "-- 拉取 myfeed 索引 packages.adb --"
    rm -f /tmp/diag-myfeed.adb
    timeout 30 wget -O /tmp/diag-myfeed.adb "$(myfeed_repo_url)" \
        2>&1 | tail -10 | tee -a "$LOG"
    if [ -s /tmp/diag-myfeed.adb ]; then
        SIZE="$(wc -c < /tmp/diag-myfeed.adb)"
        log "  index OK: $SIZE 字节（说明 DNS/TCP/HTTPS 通，问题在单个 .apk 文件下载）"
    else
        log "  index 失败：DNS/TCP/HTTPS 三层之一不通"
    fi
    rm -f /tmp/diag-myfeed.adb

    log "-- apk fetch --simulate 测一个 myfeed 包 --"
    apk fetch --simulate luci-app-bandix@myfeed 2>&1 | tail -10 | tee -a "$LOG"

    log "-- /etc/resolv.conf --"
    cat /etc/resolv.conf 2>&1 | tee -a "$LOG"

    log "-- 当前 DNS 服务状态 --"
    for SVC in smartdns dnsmasq; do
        if [ -x "/etc/init.d/$SVC" ]; then
            STATE="stopped"
            /etc/init.d/"$SVC" running >/dev/null 2>&1 && STATE="running"
            log "  $SVC: $STATE"
        fi
    done
    log "===== 诊断结束 ====="
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

keep_backup_pkg() {
    NAME="${1%@*}"
    NAME="${NAME%%[<>=~]*}"

    for KEEP in $RESTORE_KEEP_BACKUP_PACKAGES; do
        [ "$NAME" = "$KEEP" ] && return 0
    done

    if [ -s /root/restore-meta/packages.preserved-by-keep-backup ] && \
       grep -qxF "$NAME" /root/restore-meta/packages.preserved-by-keep-backup; then
        return 0
    fi

    if [ -s /root/restore-meta/packages.keep-backup-roots ] && \
       grep -qxF "$NAME" /root/restore-meta/packages.keep-backup-roots; then
        return 0
    fi

    return 1
}

restore_keep_backup_world_entries() {
    [ -s /root/restore-meta/world.keep-backup ] || return 0
    [ -f /etc/apk/world ] || : > /etc/apk/world

    WORLD_TMP=/tmp/world-keep-backup.$$
    {
        cat /etc/apk/world 2>/dev/null
        cat /root/restore-meta/world.keep-backup
    } | sed '/^$/d' | sort -u > "$WORLD_TMP"

    if cmp -s /etc/apk/world "$WORLD_TMP" 2>/dev/null; then
        rm -f "$WORLD_TMP"
        return 0
    fi

    mv "$WORLD_TMP" /etc/apk/world
    log "已保留备份中的 Nikki/指定保留包 world 条目，不通过 apk 重装覆盖"
}

keep_myfeed_backup_when_feed_down() {
    [ "$RESTORE_KEEP_MYFEED_WHEN_FEED_DOWN" = "1" ] || return 1
    myfeed_seems_dead || return 1

    NAME="${1%@*}"
    NAME="${NAME%%[<>=~]*}"
    is_myfeed_pkg "$NAME"
}

target_uses_myfeed() {
    TARGET="$1"
    BASE="${TARGET%@*}"

    case "$TARGET" in
        *@myfeed) return 0 ;;
    esac

    is_myfeed_pkg "$BASE"
}

list_all_targets_use_myfeed() {
    TARGET_LIST="$1"
    [ -s "$TARGET_LIST" ] || return 1

    while read TARGET; do
        [ -n "$TARGET" ] || continue
        target_uses_myfeed "$TARGET" || return 1
    done < "$TARGET_LIST"

    return 0
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

emit_speedtest_target() {
    if repo_has_pkg speedtest-cli; then
        resolve_install_name speedtest-cli
        return $?
    fi

    if apk info -e naiveproxy >/dev/null 2>&1 || [ -e /usr/bin/naive ]; then
        log "SKIP speedtest-go: conflicts with existing naiveproxy /usr/bin/naive" >&2
        return 0
    fi

    emit_first_available_target speedtest-go
}

resolve_install_name() {
    NAME="${1%@*}"

    if is_myfeed_pkg "$NAME"; then
        if policy_has_tag "$NAME" "@myfeed"; then
            echo "${NAME}@myfeed"
            return 0
        fi

        log "NOT_IN_MYFEED: $NAME" >&2
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
                               emit_speedtest_target || return 1
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

remove_pkg_family_from_world() {
    PKG="$1"
    [ -n "$PKG" ] || return 0
    [ -f /etc/apk/world ] || return 0

    BASE="${PKG%@*}"
    BASE="${BASE%%[<>=~]*}"
    [ -n "$BASE" ] || return 0

    WORLD_TMP=/tmp/world-family.$$
    awk -v drop="$BASE" '
        {
            item = $0
            sub(/@.*/, "", item)
            sub(/[<>=~].*/, "", item)
            if (item == drop)
                next
            print
        }
    ' /etc/apk/world > "$WORLD_TMP"
    mv "$WORLD_TMP" /etc/apk/world
}

strip_myfeed_tags_from_world() {
    [ -f /etc/apk/world ] || return 0

    mkdir -p /root/restore-meta
    WORLD_TMP=/tmp/world-strip-myfeed.$$
    sed 's/@myfeed//g' /etc/apk/world > "$WORLD_TMP"

    if cmp -s /etc/apk/world "$WORLD_TMP" 2>/dev/null; then
        rm -f "$WORLD_TMP"
        return 0
    fi

    cp /etc/apk/world /root/restore-meta/world.before-myfeed-luci-compat 2>/dev/null || true
    mv "$WORLD_TMP" /etc/apk/world
    log "已移除 apk world 中的 @myfeed 标签，避免 LuCI/普通 apk add 被缺失 tag 阻塞"
}

retag_myfeed_world_entries() {
    [ -f /etc/apk/world ] || return 0

    mkdir -p /root/restore-meta
    WORLD_TMP=/tmp/world-retag-myfeed.$$
    awk -v re="$MYFEED_RE" '
        {
            item = $0
            sub(/@.*/, "", item)
            sub(/[<>=~].*/, "", item)
            if (item ~ re)
                print item "@myfeed"
            else
                print $0
        }
    ' /etc/apk/world | sort -u > "$WORLD_TMP"

    if cmp -s /etc/apk/world "$WORLD_TMP" 2>/dev/null; then
        rm -f "$WORLD_TMP"
        return 0
    fi

    cp /etc/apk/world /root/restore-meta/world.before-myfeed-retag 2>/dev/null || true
    mv "$WORLD_TMP" /etc/apk/world
    log "已把 apk world 中的 myfeed 根包恢复为 @myfeed tagged 形式"
}

remove_myfeed_roots_from_world() {
    [ -f /etc/apk/world ] || return 0

    mkdir -p /root/restore-meta
    WORLD_TMP=/tmp/world-remove-myfeed.$$
    REMOVED=/root/restore-meta/world.removed-myfeed-disabled
    : > "$REMOVED"

    awk -v re="$MYFEED_RE" -v removed="$REMOVED" '
        {
            item = $0
            sub(/@.*/, "", item)
            sub(/[<>=~].*/, "", item)
            if (item ~ re) {
                print item >> removed
                next
            }
            print $0
        }
    ' /etc/apk/world > "$WORLD_TMP"

    mv "$WORLD_TMP" /etc/apk/world

    if [ -s "$REMOVED" ]; then
        sort -u "$REMOVED" > "$REMOVED.tmp"
        mv "$REMOVED.tmp" "$REMOVED"
        log "myfeed 已禁用，从 apk world 移除 myfeed 根包，避免后续安装被不可用源阻塞："
        cat "$REMOVED" | tee -a "$LOG"
    fi
}

target_installed() {
    BASE="${1%@*}"
    apk info -e "$BASE" >/dev/null 2>&1
}

remove_target_from_world_if_missing() {
    TARGET="$1"
    BASE="${TARGET%@*}"

    target_installed "$TARGET" && return 0
    remove_from_world "$BASE"
    remove_from_world "$TARGET"
}

record_failed_target() {
    TARGET="$1"
    BASE="${TARGET%@*}"

    log "FAILED: $TARGET"
    echo "$BASE" >> "$FAILED"
    remove_from_world "$BASE"
    remove_from_world "$TARGET"
}

remove_missing_targets_from_world() {
    TARGET_LIST="$1"
    [ -s "$TARGET_LIST" ] || return 0

    while read TARGET; do
        [ -n "$TARGET" ] || continue
        remove_target_from_world_if_missing "$TARGET"
    done < "$TARGET_LIST"
}

is_luci_target() {
    BASE="${1%@*}"
    echo "$BASE" | grep -Eq '^luci-(app|i18n)-'
}

retry_missing_targets() {
    TARGET_LIST="$1"
    [ -s "$TARGET_LIST" ] || return 0

    if myfeed_seems_dead && list_all_targets_use_myfeed "$TARGET_LIST"; then
        announce_myfeed_dead
        log "myfeed 早停，跳过逐个重试，从 world 中清理未安装项"
        while read TARGET; do
            [ -n "$TARGET" ] || continue
            target_installed "$TARGET" && continue
            remove_target_from_world_if_missing "$TARGET"
        done < "$TARGET_LIST"
        return 0
    fi

    if network_seems_dead; then
        announce_network_dead
        log "网络早停，跳过逐个重试，从 world 中清理未安装项"
        while read TARGET; do
            [ -n "$TARGET" ] || continue
            target_installed "$TARGET" && continue
            remove_target_from_world_if_missing "$TARGET"
        done < "$TARGET_LIST"
        return 0
    fi

    log "批量安装失败，逐个重试未安装的软件包（单次尝试，不再嵌套 3 次）..."
    remove_missing_targets_from_world "$TARGET_LIST"

    while read TARGET; do
        [ -n "$TARGET" ] || continue
        target_installed "$TARGET" && continue

        if myfeed_seems_dead && target_uses_myfeed "$TARGET"; then
            announce_myfeed_dead
            log "myfeed 早停，跳过逐个重试: $TARGET"
            remove_target_from_world_if_missing "$TARGET"
            continue
        fi

        if network_seems_dead; then
            log "网络早停，中断剩余逐个重试"
            remove_target_from_world_if_missing "$TARGET"
            continue
        fi

        log "RETRY: $TARGET"
        remove_target_from_world_if_missing "$TARGET"
        apk_add_safe "$APK_ADD_RETRY_TIMEOUT" "$TARGET" >>"$LOG" 2>&1 || {
            log "RETRY FAILED: $TARGET"
            remove_target_from_world_if_missing "$TARGET"
        }
        check_network_fatigue
        announce_myfeed_dead
    done < "$TARGET_LIST"
}

install_target_group() {
    TARGET_LIST="$1"
    LABEL="$2"
    LIMIT="$3"
    RETRY="$4"

    [ -s "$TARGET_LIST" ] || return 0

    N="$(wc -l < "$TARGET_LIST")"

    if myfeed_seems_dead && list_all_targets_use_myfeed "$TARGET_LIST"; then
        announce_myfeed_dead
        log "myfeed 早停，跳过批量安装 $N 个${LABEL}"
        while read TARGET; do
            [ -n "$TARGET" ] || continue
            target_installed "$TARGET" || record_failed_target "$TARGET"
        done < "$TARGET_LIST"
        return 0
    fi

    if network_seems_dead; then
        announce_network_dead
        log "网络早停，跳过批量安装 $N 个${LABEL}"
        while read TARGET; do
            [ -n "$TARGET" ] || continue
            target_installed "$TARGET" || record_failed_target "$TARGET"
        done < "$TARGET_LIST"
        return 0
    fi

    log "批量安装 $N 个${LABEL}..."
    if ! apk_add_retry "$LIMIT" $(cat "$TARGET_LIST") >>"$LOG" 2>&1; then
        log "WARNING: ${LABEL}批量 apk add 返回非零，详情见 $LOG"
        check_network_fatigue
        announce_myfeed_dead
        announce_network_dead
        remove_missing_targets_from_world "$TARGET_LIST"
        if [ "$RETRY" = "1" ]; then
            retry_missing_targets "$TARGET_LIST"
        else
            log "跳过 ${LABEL}逐个重试，避免单个 LuCI 页面包长时间卡住"
        fi
    fi

    while read TARGET; do
        [ -n "$TARGET" ] || continue
        target_installed "$TARGET" || record_failed_target "$TARGET"
    done < "$TARGET_LIST"
}

install_target_list() {
    LIST_FILE="$1"
    LIST_LABEL="$2"
    LIST_LIMIT="$3"
    LIST_RETRY="$4"
    LIST_CHUNK="${5:-0}"

    [ -s "$LIST_FILE" ] || return 0

    if [ "$LIST_CHUNK" -le 0 ] 2>/dev/null; then
        install_target_group "$LIST_FILE" "$LIST_LABEL" "$LIST_LIMIT" "$LIST_RETRY"
        return 0
    fi

    TOTAL="$(wc -l < "$LIST_FILE")"
    log "分批安装 $TOTAL 个${LIST_LABEL}（每批最多 $LIST_CHUNK 个）..."

    BATCH_FILE="/tmp/restore-install-batch.$$.txt"
    : > "$BATCH_FILE"
    BATCH_COUNT=0
    BATCH_NO=1

    while read TARGET; do
        [ -n "$TARGET" ] || continue
        echo "$TARGET" >> "$BATCH_FILE"
        BATCH_COUNT=$((BATCH_COUNT + 1))

        if [ "$BATCH_COUNT" -ge "$LIST_CHUNK" ]; then
            install_target_group "$BATCH_FILE" "${LIST_LABEL} #$BATCH_NO" "$LIST_LIMIT" "$LIST_RETRY"
            : > "$BATCH_FILE"
            BATCH_COUNT=0
            BATCH_NO=$((BATCH_NO + 1))
        fi
    done < "$LIST_FILE"

    if [ -s "$BATCH_FILE" ]; then
        install_target_group "$BATCH_FILE" "${LIST_LABEL} #$BATCH_NO" "$LIST_LIMIT" "$LIST_RETRY"
    fi

    rm -f "$BATCH_FILE"
}

install_deferred_proxy_runtime() {
    [ -s "$DEFERRED_PROXY_RUNTIME_LIST" ] || return 0

    sort -u "$DEFERRED_PROXY_RUNTIME_LIST" > "$DEFERRED_PROXY_RUNTIME_LIST.sorted"
    mv "$DEFERRED_PROXY_RUNTIME_LIST.sorted" "$DEFERRED_PROXY_RUNTIME_LIST"

    if [ "$UPDATE_OK" != "1" ]; then
        log "跳过恢复代理后端补装：apk update 未成功"
        return 0
    fi

    N="$(wc -l < "$DEFERRED_PROXY_RUNTIME_LIST")"

    if myfeed_seems_dead && list_all_targets_use_myfeed "$DEFERRED_PROXY_RUNTIME_LIST"; then
        announce_myfeed_dead
        log "myfeed 早停，跳过 $N 个恢复代理后端包补装，保留旧运行时"
        while read TARGET; do
            [ -n "$TARGET" ] || continue
            record_failed_target "$TARGET"
        done < "$DEFERRED_PROXY_RUNTIME_LIST"
        return 0
    fi

    if network_seems_dead; then
        announce_network_dead
        log "网络早停，跳过 $N 个恢复代理后端包补装，保留旧运行时"
        while read TARGET; do
            [ -n "$TARGET" ] || continue
            record_failed_target "$TARGET"
        done < "$DEFERRED_PROXY_RUNTIME_LIST"
        return 0
    fi

    log "主要软件恢复完成，补装/修复 $N 个恢复代理后端包..."

    if apk_add_retry "$APK_ADD_BATCH_TIMEOUT" $(cat "$DEFERRED_PROXY_RUNTIME_LIST") >>"$LOG" 2>&1; then
        log "恢复代理后端包补装/修复成功"
        return 0
    fi

    log "WARNING: 恢复代理后端包补装/修复失败，保留旧运行时继续使用"
    while read TARGET; do
        [ -n "$TARGET" ] || continue
        record_failed_target "$TARGET"
    done < "$DEFERRED_PROXY_RUNTIME_LIST"
}

retry_failed_packages_before_report() {
    [ -s "$FAILED" ] || return 0
    [ "$UPDATE_OK" = "1" ] || return 0

    RETRY_LIST=/tmp/restore-final-failed-retry.$$.txt
    : > "$RETRY_LIST"

    sort -u "$FAILED" | while read PKG; do
        [ -n "$PKG" ] || continue
        apk info -e "$PKG" >/dev/null 2>&1 && continue
        repo_has_pkg "$PKG" || continue
        echo "$PKG"
    done > "$RETRY_LIST"

    if [ -s "$RETRY_LIST" ]; then
        log "最终补装失败列表中仍未安装但源里存在的软件包..."
        apk_add_retry "$APK_ADD_RETRY_TIMEOUT" $(cat "$RETRY_LIST") >>"$LOG" 2>&1 || \
            log "WARNING: final failed-package retry returned non-zero"
    fi

    rm -f "$RETRY_LIST"
}

restart_restore_proxy_backends() {
    [ "$RESTORE_PROXY_UP" = "1" ] || return 0

    for SVC in smartdns nikki; do
        [ -x "/etc/init.d/$SVC" ] || continue
        log "重启恢复代理后端: $SVC"
        /etc/init.d/"$SVC" restart >>"$LOG" 2>&1 || \
            /etc/init.d/"$SVC" start >>"$LOG" 2>&1 || \
            log "WARNING: 恢复代理后端重启失败: $SVC"
    done
}

drop_unwanted_luci_pages() {
    DROP_LIST=/tmp/restore-drop-luci-list.txt
    apk info 2>/dev/null | grep -E "$DROP_LUCI_RE" | sort -u > "$DROP_LIST" || true

    if [ -s "$DROP_LIST" ]; then
        N="$(wc -l < "$DROP_LIST")"
        log "批量移除 $N 个不需要的 LuCI 页面..."
        cat "$DROP_LIST" | tee -a "$LOG"

        apk del --force-broken-world $(cat "$DROP_LIST") >>"$LOG" 2>&1 || \
            log "WARNING: batch luci page removal returned non-zero"

        while read PKG; do
            [ -n "$PKG" ] || continue
            remove_from_world "$PKG"
        done < "$DROP_LIST"
    fi

    strip_world_drop_pattern
}

strip_world_drop_pattern() {
    [ -f /etc/apk/world ] || return 0
    WORLD_TMP=/tmp/restore-world-drop.$$
    DROPPED=0
    while read PKG; do
        [ -n "$PKG" ] || continue
        BASE="${PKG%@*}"
        BASE="${BASE%%[<>=~]*}"
        if echo "$BASE" | grep -Eq "$DROP_LUCI_RE"; then
            DROPPED=$((DROPPED + 1))
            continue
        fi
        echo "$PKG"
    done < /etc/apk/world > "$WORLD_TMP"

    if [ "$DROPPED" -gt 0 ]; then
        log "从 world 移除 $DROPPED 个未安装但匹配 DROP_LUCI 的项"
        mv "$WORLD_TMP" /etc/apk/world
    else
        rm -f "$WORLD_TMP"
    fi
}

strip_world_unsatisfiable() {
    [ -f /etc/apk/world ] || return 0
    log "清理 world 中既未安装、源里也没有的软件包..."

    WORLD_TMP=/tmp/restore-world-check.$$
    DROPPED=0

    while read PKG; do
        [ -n "$PKG" ] || continue
        case "$PKG" in
            kernel=*|kmod-*) echo "$PKG"; continue ;;
        esac
        BASE="${PKG%@*}"
        BASE="${BASE%%[<>=~]*}"

        if apk info -e "$BASE" >/dev/null 2>&1; then
            echo "$PKG"
            continue
        fi

        if repo_has_pkg "$BASE"; then
            echo "$PKG"
            continue
        fi

        log "  world 不可满足，移除: $PKG"
        DROPPED=$((DROPPED + 1))
    done < /etc/apk/world > "$WORLD_TMP"

    if [ "$DROPPED" -gt 0 ]; then
        mv "$WORLD_TMP" /etc/apk/world
    else
        rm -f "$WORLD_TMP"
    fi
}

quarantine_failed_world_entries() {
    QUARANTINED=/root/restore-meta/world.removed-failed-packages
    mkdir -p /root/restore-meta
    : > "$QUARANTINED"

    cat "$FAILED" "$NOT_IN_REPO" 2>/dev/null | sort -u | while read PKG; do
        [ -n "$PKG" ] || continue
        echo "$PKG" >> "$QUARANTINED"
        remove_pkg_family_from_world "$PKG"
    done

    if [ -s "$QUARANTINED" ]; then
        log "已从 apk world 隔离失败/缺源包，避免影响后续安装："
        cat "$QUARANTINED" | tee -a "$LOG"
    fi
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

restore_luci_wrapper() {
    [ "$RESTORE_LUCI_WRAPPER" = "1" ] || {
        log "RESTORE_LUCI_WRAPPER=0：跳过 LuCI wrapper 修复"
        return 0
    }

    mkdir -p /www/cgi-bin /root/restore-meta
    CHANGED=0

    if [ ! -f /www/index.html ] || ! grep -q 'cgi-bin/luci' /www/index.html 2>/dev/null; then
        [ -f /www/index.html ] && \
            cp /www/index.html /root/restore-meta/www.index.before-luci-wrapper 2>/dev/null || true

        cat > /www/index.html <<'LUCIINDEXEOF'
<!DOCTYPE html>
<html>
	<head>
		<meta http-equiv="Cache-Control" content="no-cache, no-store, must-revalidate" />
		<meta http-equiv="Pragma" content="no-cache" />
		<meta http-equiv="Expires" content="0" />
		<meta http-equiv="Expires" content="Thu, 01 Jan 1970 00:00:00 GMT" />
		<meta http-equiv="refresh" content="0; URL=cgi-bin/luci/" />
		<style>
			body { background: white; font-family: arial, helvetica, sans-serif; }
			a { color: black; }

			@media (prefers-color-scheme: dark) {
				body { background: black; }
				a { color: white; }
			}
		</style>
	</head>
	<body>
		<a href="cgi-bin/luci/">LuCI - Lua Configuration Interface</a>
	</body>
</html>
LUCIINDEXEOF
        CHANGED=1
    fi

    if [ ! -x /www/cgi-bin/luci ]; then
        if command -v ucode >/dev/null 2>&1; then
            cat > /www/cgi-bin/luci <<'LUCIUCODEEOF'
#!/usr/bin/env ucode

'use strict';

import { stdin, stdout } from 'fs';

import dispatch from 'luci.dispatcher';
import request from 'luci.http';

const input_bufsize = 4096;
let input_available = +getenv('CONTENT_LENGTH') || 0;

function read(len) {
	if (input_available == 0) {
		stdin.close();

		return null;
	}

	let chunk = stdin.read(min(input_available, len ?? input_bufsize, input_bufsize));

	if (chunk == null) {
		input_available = 0;
		stdin.close();
	}
	else {
		input_available -= length(chunk);
	}

	return chunk;
}

function write(data) {
	return stdout.write(data);
}

let req = request(getenv(), read, write);

dispatch(req);

req.close();
LUCIUCODEEOF
            chmod 755 /www/cgi-bin/luci
            CHANGED=1
        elif [ -x /usr/bin/lua ] && [ -d /usr/lib/lua/luci ]; then
            cat > /www/cgi-bin/luci <<'LUCILUAEOF'
#!/usr/bin/lua
require "luci.cacheloader"
require "luci.sgi.cgi"
luci.dispatcher.indexcache = "/tmp/luci-indexcache"
luci.sgi.cgi.run()
LUCILUAEOF
            chmod 755 /www/cgi-bin/luci
            CHANGED=1
        else
            log "WARNING: LuCI wrapper 缺失，但未找到 ucode 或 Lua LuCI 运行时，跳过生成 /www/cgi-bin/luci"
        fi
    fi

    if [ -f /etc/config/uhttpd ] && command -v uci >/dev/null 2>&1; then
        if ! uci -q get uhttpd.main.cgi_prefix >/dev/null 2>&1; then
            uci set uhttpd.main.cgi_prefix='/cgi-bin' 2>/dev/null || true
            uci commit uhttpd 2>/dev/null || true
            CHANGED=1
        fi
    fi

    if [ "$CHANGED" -gt 0 ]; then
        log "已补齐 LuCI wrapper 入口"
    else
        log "LuCI wrapper 入口已存在"
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

    mkdir -p /etc/apk/keys /etc/apk/repositories.d

    for BASE in $MYFEED_BASES; do
        [ -n "$BASE" ] || continue
        MYFEED_BASE="${BASE%/}"
        MY_REPO="$(myfeed_repo_url)"

        log "检查 myfeed 镜像: $MYFEED_BASE"
        pin_myfeed_ipv4

        wget -O /etc/apk/keys/myfeed.pem "$(myfeed_key_url)" >>"$LOG" 2>&1 || {
            log "WARNING: myfeed key download failed: $MYFEED_BASE"
            rm -f /etc/apk/keys/myfeed.pem
            continue
        }

        rm -f /tmp/restore-myfeed-index-test.adb
        timeout 30 wget -O /tmp/restore-myfeed-index-test.adb "$MY_REPO" >>"$LOG" 2>&1 || {
            log "WARNING: myfeed index download failed: $MY_REPO"
            rm -f /etc/apk/keys/myfeed.pem /tmp/restore-myfeed-index-test.adb
            continue
        }

        if [ ! -s /tmp/restore-myfeed-index-test.adb ]; then
            log "WARNING: myfeed index is empty: $MY_REPO"
            rm -f /etc/apk/keys/myfeed.pem /tmp/restore-myfeed-index-test.adb
            continue
        fi

        rm -f /tmp/restore-myfeed-index-test.adb
        echo "@myfeed $MY_REPO" > /etc/apk/repositories.d/00-myfeed.list
        log "Added myfeed (tagged @myfeed): $MY_REPO"
        return 0
    done

    MYFEED_BASE=""
    MYFEED_DEAD=1
    rm -f /etc/apk/keys/myfeed.pem
    log "WARNING: 所有 myfeed 镜像都不可用，跳过 myfeed 源"
}

enable_myfeed_luci_compat() {
    [ "$MYFEED_LUCI_COMPAT" = "1" ] || {
        log "MYFEED_LUCI_COMPAT=0：保留 tagged @myfeed 源"
        return 0
    }

    FILE=/etc/apk/repositories.d/00-myfeed.list
    [ -f "$FILE" ] || return 0

    MY_REPO="$(sed -n 's/^@myfeed[[:space:]]\{1,\}//p' "$FILE" | head -n 1)"
    if [ -z "$MY_REPO" ]; then
        MY_REPO="$(sed -n '/^[[:space:]]*#/d; /^[[:space:]]*$/d; p' "$FILE" | head -n 1)"
        [ -n "$MY_REPO" ] || return 0
        log "myfeed 已是 LuCI 兼容的未 tagged 源，复查 apk update"
    fi

    mkdir -p /root/restore-meta
    cp "$FILE" /root/restore-meta/00-myfeed.before-luci-compat 2>/dev/null || true
    echo "$MY_REPO" > "$FILE"
    strip_myfeed_tags_from_world
    log "已切换 myfeed 为 LuCI 兼容源：LuCI 软件包管理器可直接安装 myfeed 包"

    if apk_update_safe >>"$LOG" 2>&1; then
        return 0
    fi

    log "WARNING: myfeed LuCI 兼容切换后 apk update 失败，回退为 tagged @myfeed 源"
    echo "@myfeed $MY_REPO" > "$FILE"
    retag_myfeed_world_entries
    if apk_update_safe >>"$LOG" 2>&1; then
        log "已回退为 tagged @myfeed；官方包可继续安装，myfeed 包请用 SSH 执行 apk add 包名@myfeed"
        return 0
    fi

    log "WARNING: tagged @myfeed 源也更新失败，临时禁用 myfeed，优先保证官方源可继续安装"
    disable_repo_file "$FILE"
    strip_myfeed_tags_from_world
    remove_myfeed_roots_from_world
    apk_update_safe >>"$LOG" 2>&1 || \
        log "WARNING: 禁用 myfeed 后 apk update 仍失败，请检查官方源或网络"
}

disable_repo_file() {
    FILE="$1"
    [ -f "$FILE" ] || return 0
    cp "$FILE" "$FILE.disabled"
    sed 's/^/# disabled by restore: /' "$FILE.disabled" > "$FILE"
    log "Disabled repo file: $FILE (backup: $FILE.disabled)"
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
bring_up_restore_proxy

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
restore_keep_backup_world_entries

drop_unwanted_luci_pages

restore_distfeeds

log "添加当前固件匹配的自用 feed 和内核模块源..."
add_myfeed

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
INSTALL_MYFEED_CORE_LIST=/tmp/restore-install-myfeed-core-list.txt
INSTALL_MYFEED_LUCI_LIST=/tmp/restore-install-myfeed-luci-list.txt
DEFERRED_PROXY_RUNTIME_LIST=/tmp/restore-deferred-proxy-runtime-list.txt
: > "$INSTALL_LIST"
: > "$INSTALL_CORE_LIST"
: > "$INSTALL_LUCI_LIST"
: > "$INSTALL_MYFEED_CORE_LIST"
: > "$INSTALL_MYFEED_LUCI_LIST"
: > "$DEFERRED_PROXY_RUNTIME_LIST"

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

        if keep_backup_pkg "$BASE"; then
            log "保留备份中的 $BASE，不通过 apk 重装覆盖"
            remove_from_world "$BASE"
            remove_from_world "$PKG"
            continue
        fi

        if keep_myfeed_backup_when_feed_down "$BASE"; then
            log "myfeed 不可用，保留备份中的 $BASE，跳过重装"
            remove_from_world "$BASE"
            remove_from_world "$PKG"
            continue
        fi

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

        if [ "$KEEP_PROXY_UP" = "1" ]; then
            FILTERED=/tmp/restore-install-list-filtered.txt
            : > "$FILTERED"
            while read TARGET; do
                [ -n "$TARGET" ] || continue
                BASE_T="${TARGET%@*}"
                if keep_backup_pkg "$BASE_T"; then
                    log "保留备份中的 $TARGET，跳过重装"
                    remove_from_world "$BASE_T"
                    remove_from_world "$TARGET"
                    continue
                fi
                case "$BASE_T" in
                    smartdns|nikki)
                        if service_running "$BASE_T"; then
                            log "保留恢复代理后端，延后补装/修复: $TARGET (LuCI 页面仍会装)"
                            echo "$TARGET" >> "$DEFERRED_PROXY_RUNTIME_LIST"
                            continue
                        fi
                        proxy_backend_available "$BASE_T" && \
                            log "恢复代理后端存在但未运行，纳入首轮重装: $TARGET"
                        ;;
                esac
                echo "$TARGET" >> "$FILTERED"
            done < "$INSTALL_LIST"
            mv "$FILTERED" "$INSTALL_LIST"
        fi

        FILTERED_KEEP=/tmp/restore-install-list-keep-backup.txt
        : > "$FILTERED_KEEP"
        while read TARGET; do
            [ -n "$TARGET" ] || continue
            BASE_T="${TARGET%@*}"
            if keep_backup_pkg "$BASE_T"; then
                log "保留备份中的 $TARGET，跳过重装"
                remove_from_world "$BASE_T"
                remove_from_world "$TARGET"
                continue
            fi
            if keep_myfeed_backup_when_feed_down "$BASE_T"; then
                log "myfeed 不可用，保留备份中的 $TARGET，跳过重装"
                remove_from_world "$BASE_T"
                remove_from_world "$TARGET"
                continue
            fi
            echo "$TARGET" >> "$FILTERED_KEEP"
        done < "$INSTALL_LIST"
        mv "$FILTERED_KEEP" "$INSTALL_LIST"

        while read TARGET; do
            [ -n "$TARGET" ] || continue
            if target_uses_myfeed "$TARGET"; then
                if is_luci_target "$TARGET"; then
                    echo "$TARGET" >> "$INSTALL_MYFEED_LUCI_LIST"
                else
                    echo "$TARGET" >> "$INSTALL_MYFEED_CORE_LIST"
                fi
            elif is_luci_target "$TARGET"; then
                echo "$TARGET" >> "$INSTALL_LUCI_LIST"
            else
                echo "$TARGET" >> "$INSTALL_CORE_LIST"
            fi
        done < "$INSTALL_LIST"

        install_target_list "$INSTALL_CORE_LIST" "官方核心/后端包" "$APK_ADD_BATCH_TIMEOUT" "1" "$APK_ADD_CORE_CHUNK_SIZE"
        install_target_list "$INSTALL_LUCI_LIST" "官方 LuCI 页面包" "$APK_ADD_LUCI_TIMEOUT" "$APK_RETRY_LUCI"
        install_target_list "$INSTALL_MYFEED_CORE_LIST" "myfeed 核心/后端包" "$APK_ADD_BATCH_TIMEOUT" "1" "$APK_ADD_CORE_CHUNK_SIZE"
        install_target_list "$INSTALL_MYFEED_LUCI_LIST" "myfeed LuCI 页面包" "$APK_ADD_LUCI_TIMEOUT" "$APK_RETRY_LUCI"
    else
        log "没有需要新装的包"
    fi
else
    [ -s "$LIST" ] || log "没有找到软件包恢复清单：$LIST"
fi

drop_unwanted_luci_pages
strip_world_unsatisfiable
repair_luci_runtime
install_deferred_proxy_runtime
restore_app_configs
restore_luci_commands
restore_luci_wrapper
restart_restore_proxy_backends
retry_failed_packages_before_report
enable_myfeed_luci_compat
restore_keep_backup_world_entries

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
quarantine_failed_world_entries
strip_world_unsatisfiable

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
