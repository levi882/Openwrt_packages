#!/bin/sh
set -e

BACKUP_FILE="$1"

[ -f "$BACKUP_FILE" ] || {
    echo "用法：restore_overlay.sh 备份文件"
    exit 1
}

echo "即将恢复：$BACKUP_FILE"
echo "最低限度清理：旧 kmod/旧 apk 状态 + SmartDNS 运行缓存"
echo "软件包文件和配置尽量保留"
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

echo "保存旧软件包清单，并过滤 kernel= 与 kmod-*..."
if [ -f "$UP/etc/apk/world" ]; then
    cp "$UP/etc/apk/world" "$UP/root/restore-meta/world.from-backup"

    sed -E '
        /^kernel=/d;
        /^kmod-/d;
        /^$/d;
        /^#/d;
        s/[<>=~].*$//
    ' "$UP/etc/apk/world" | sort -u > "$UP/root/apk-world.restore-list"
fi

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

remove_from_world() {
    PKG="$1"
    [ -n "$PKG" ] || return 0
    [ -f /etc/apk/world ] || return 0
    grep -vxF "$PKG" /etc/apk/world > /tmp/world.$$ 2>/dev/null || true
    mv /tmp/world.$$ /etc/apk/world
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

    echo "$MY_REPO" > /etc/apk/repositories.d/00-myfeed.list
    log "Added myfeed: $MY_REPO"
}

add_nikki_feed_if_needed() {
    list_has_pkg_pattern '^(nikki|luci-app-nikki|mihomo-meta|mihomo-alpha|luci-i18n-nikki-)' || return 0

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

    echo "$NIKKI_REPO" > /etc/apk/repositories.d/20-nikki.list
    log "Added Nikki feed: $NIKKI_REPO"
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

: > "$LOG"
: > "$FAILED"
: > "$NOT_IN_REPO"

log "== post restore reinstall start =="

[ -x /etc/init.d/fakehttp ] && /etc/init.d/fakehttp stop 2>/dev/null || true

log "恢复当前固件 apk world 和源..."
[ -f /rom/etc/apk/world ] && cp /rom/etc/apk/world /etc/apk/world

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
fi

if [ "$UPDATE_OK" = "1" ] && [ -s "$LIST" ]; then
    TOTAL="$(wc -l < "$LIST")"
    i=0

    log "按旧清单恢复普通软件包，已过滤 kmod-*..."
    while read PKG; do
        [ -n "$PKG" ] || continue
        i=$((i + 1))

        apk info -e "$PKG" >/dev/null 2>&1 && continue

        if ! repo_has_pkg "$PKG"; then
            log "SKIP not in repo: $PKG"
            echo "$PKG" >> "$NOT_IN_REPO"
            remove_from_world "$PKG"
            continue
        fi

        log "[$i/$TOTAL] Installing $PKG"
        apk add --force-broken-world "$PKG" >>"$LOG" 2>&1 || {
            log "FAILED: $PKG"
            echo "$PKG" >> "$FAILED"
            remove_from_world "$PKG"
        }
    done < "$LIST"
else
    [ -s "$LIST" ] || log "没有找到软件包恢复清单：$LIST"
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
/etc/init.d/rpcd restart 2>/dev/null || true
/etc/init.d/uhttpd restart 2>/dev/null || true

if [ -x /etc/init.d/smartdns ]; then
    /etc/init.d/smartdns restart 2>/dev/null || true
fi

if [ -x /etc/init.d/fakehttp ]; then
    /etc/init.d/fakehttp enable 2>/dev/null || true
    /etc/init.d/fakehttp restart 2>/dev/null || true
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
