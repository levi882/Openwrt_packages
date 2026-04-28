#!/bin/sh
set -e

BACKUP_FILE="$1"
RESTORE_KEEP_EXTROOT="${RESTORE_KEEP_EXTROOT:-0}"
DEFAULT_KEEP_RUNTIME_PACKAGES="smartdns nikki"
RESTORE_KEEP_RUNTIME_PACKAGES="${RESTORE_KEEP_RUNTIME_PACKAGES-$DEFAULT_KEEP_RUNTIME_PACKAGES}"
RESTORE_MYFEED_BASE="${RESTORE_MYFEED_BASE:-https://openwrt-packages.pages.dev}"
RESTORE_MYFEED_REPO="${RESTORE_MYFEED_REPO:-$RESTORE_MYFEED_BASE/openwrt-25.12/x86_64/myfeed/packages.adb}"
RESTORE_MYFEED_KEY_URL="${RESTORE_MYFEED_KEY_URL:-$RESTORE_MYFEED_BASE/public-key.pem}"
DEFAULT_INSTALL_PACKAGES="omcproxy luci-app-omcproxy luci-i18n-omcproxy-zh-cn"
RESTORE_INSTALL_PACKAGES="${RESTORE_INSTALL_PACKAGES-$DEFAULT_INSTALL_PACKAGES}"
DEFAULT_MYFEED_INSTALL_PACKAGES="bandix luci-app-bandix luci-i18n-bandix-zh-cn easytier luci-app-easytier luci-i18n-easytier-zh-cn fakehttp luci-app-fakehttp luci-i18n-fakehttp-zh-cn lucky luci-app-lucky luci-i18n-lucky-zh-cn nikki luci-app-nikki luci-i18n-nikki-zh-cn rtp2httpd luci-app-rtp2httpd luci-i18n-rtp2httpd-zh-cn smartdns luci-app-smartdns"
RESTORE_MYFEED_INSTALL_PACKAGES="${RESTORE_MYFEED_INSTALL_PACKAGES-$DEFAULT_MYFEED_INSTALL_PACKAGES}"
DEFAULT_REMOVE_PREINSTALLED_LUCI_PACKAGES="luci-app-usb-printer luci-i18n-usb-printer-zh-cn luci-app-p910nd luci-i18n-p910nd-zh-cn luci-app-nlbwmon luci-i18n-nlbwmon-zh-cn luci-app-eqos luci-i18n-eqos-zh-cn luci-app-sqm luci-i18n-sqm-zh-cn luci-app-passwall luci-i18n-passwall-zh-cn luci-app-homeproxy luci-i18n-homeproxy-zh-cn luci-app-qbittorrent luci-i18n-qbittorrent-zh-cn luci-app-mosdns luci-i18n-mosdns-zh-cn luci-app-ddns luci-i18n-ddns-zh-cn luci-app-airconnect luci-i18n-airconnect-zh-cn luci-app-airplay2 luci-i18n-airplay2-zh-cn luci-app-frpc luci-i18n-frpc-zh-cn luci-app-mentohust luci-i18n-mentohust-zh-cn luci-app-natmap luci-i18n-natmap-zh-cn luci-app-openlist2 luci-i18n-openlist2-zh-cn luci-app-openlist luci-i18n-openlist-zh-cn luci-app-socat luci-i18n-socat-zh-cn luci-app-wolplus luci-i18n-wolplus-zh-cn luci-app-zerotier luci-i18n-zerotier-zh-cn luci-proto-wireguard luci-i18n-proto-wireguard-zh-cn luci-theme-argon luci-app-argon-config luci-i18n-argon-config-zh-cn"
RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES="${RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES-$DEFAULT_REMOVE_PREINSTALLED_LUCI_PACKAGES}"
UP=/overlay/upper
FSTAB_BEFORE_RESTORE=/tmp/restore-fstab.before-restore
DISTFEEDS_BEFORE_RESTORE=/tmp/restore-distfeeds.before-restore
MYFEED_BEFORE_RESTORE=/tmp/restore-myfeed.before-restore
MYFEED_KEY_BEFORE_RESTORE=/tmp/restore-myfeed-key.before-restore

[ -f "$BACKUP_FILE" ] || {
    echo "用法：restore_overlay.sh 备份文件"
    exit 1
}

echo "即将恢复配置：$BACKUP_FILE"
echo "只执行四件事："
echo "1. 清空 overlay 并恢复备份里的配置"
echo "2. 删除备份带来的旧内核模块和旧包管理状态"
echo "3. 删除备份带来的旧 LuCI 运行时/缓存/入口文件，让新固件使用自己的 LuCI"
echo "4. 安排重启后删除指定的新固件预装 LuCI 页面包"
echo
echo "不会添加新 feed；重启后安装自用包时会临时给 myfeed 加 @myfeed，装完改回普通源。"
echo "从备份临时保留运行时的软件包：${RESTORE_KEEP_RUNTIME_PACKAGES:-<无>}"
echo "myfeed 源：${RESTORE_MYFEED_REPO:-<无>}"
echo "重启后从当前软件源安装的软件包：${RESTORE_INSTALL_PACKAGES:-<无>}"
echo "重启后强制从 myfeed 安装的软件包：${RESTORE_MYFEED_INSTALL_PACKAGES:-<无>}"
echo "默认删除的新固件预装 LuCI 页面包：$RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES"
echo "输入 YES 继续："
read CONFIRM
[ "$CONFIRM" = "YES" ] || exit 0

rm -f "$FSTAB_BEFORE_RESTORE"
if [ "$RESTORE_KEEP_EXTROOT" != "1" ] && [ -f /etc/config/fstab ]; then
    cp /etc/config/fstab "$FSTAB_BEFORE_RESTORE"
fi

rm -f "$DISTFEEDS_BEFORE_RESTORE"
if [ -f /etc/apk/repositories.d/distfeeds.list ]; then
    cp /etc/apk/repositories.d/distfeeds.list "$DISTFEEDS_BEFORE_RESTORE"
fi

rm -f "$MYFEED_BEFORE_RESTORE" "$MYFEED_KEY_BEFORE_RESTORE"
if [ -f /etc/apk/repositories.d/00-myfeed.list ]; then
    cp /etc/apk/repositories.d/00-myfeed.list "$MYFEED_BEFORE_RESTORE"
fi
if [ -f /etc/apk/keys/myfeed.pem ]; then
    cp /etc/apk/keys/myfeed.pem "$MYFEED_KEY_BEFORE_RESTORE"
fi

echo "校验备份包..."
gzip -t "$BACKUP_FILE"
tar -tzf "$BACKUP_FILE" >/dev/null

echo "清空 overlay 并恢复备份..."
rm -rf /overlay/*
tar -xzf "$BACKUP_FILE" -C /

rm -rf "$UP/root/restore-meta"
mkdir -p "$UP/root/restore-meta"

merge_fstab_keep_current_extroot() {
    FSTAB="$UP/etc/config/fstab"

    if [ "$RESTORE_KEEP_EXTROOT" = "1" ]; then
        echo "RESTORE_KEEP_EXTROOT=1：保留备份中的完整 fstab，包括备份里的 extroot"
        return 0
    fi

    [ -s "$FSTAB_BEFORE_RESTORE" ] || return 0

    mkdir -p "$UP/etc/config" "$UP/root/restore-meta"
    [ -f "$FSTAB" ] && cp "$FSTAB" "$UP/root/restore-meta/fstab.from-backup"
    cp "$FSTAB_BEFORE_RESTORE" "$UP/root/restore-meta/fstab.current-before-restore"

    FSTAB_TMP="$FSTAB.tmp"
    : > "$FSTAB_TMP"

    if [ -f "$FSTAB" ]; then
        awk '
            function flush(    i) {
                if (n == 0) return
                if (!is_extroot) {
                    for (i = 1; i <= n; i++) print lines[i]
                }
                n = 0
                is_extroot = 0
            }
            /^[ \t]*config[ \t]+/ { flush() }
            {
                lines[++n] = $0
                if ($0 ~ /^[ \t]*option[ \t]+target[ \t]+/) {
                    target = $0
                    sub(/^[ \t]*option[ \t]+target[ \t]+/, "", target)
                    gsub(/\047/, "", target)
                    gsub(/"/, "", target)
                    gsub(/^[ \t]+|[ \t]+$/, "", target)
                    if (target == "/overlay" || target == "/") is_extroot = 1
                }
            }
            END { flush() }
        ' "$FSTAB" >> "$FSTAB_TMP"
    fi

    awk '
        function flush(    i) {
            if (n == 0) return
            if (is_extroot) {
                for (i = 1; i <= n; i++) print lines[i]
            }
            n = 0
            is_extroot = 0
        }
        /^[ \t]*config[ \t]+/ { flush() }
        {
            lines[++n] = $0
            if ($0 ~ /^[ \t]*option[ \t]+target[ \t]+/) {
                target = $0
                sub(/^[ \t]*option[ \t]+target[ \t]+/, "", target)
                gsub(/\047/, "", target)
                gsub(/"/, "", target)
                gsub(/^[ \t]+|[ \t]+$/, "", target)
                if (target == "/overlay" || target == "/") is_extroot = 1
            }
        }
        END { flush() }
    ' "$FSTAB_BEFORE_RESTORE" >> "$FSTAB_TMP"

    mv "$FSTAB_TMP" "$FSTAB"
    echo "已恢复备份中的普通挂载点，并保留当前系统的新 extroot (/overlay 或 /)"
}

merge_fstab_keep_current_extroot

prune_backup_package_files() {
    DB="$UP/lib/apk/db/installed"
    [ -s "$DB" ] || {
        echo "备份中没有 apk installed 数据库，无法清理旧软件包文件"
        return 0
    }

    mkdir -p "$UP/root/restore-meta"
    KEEP_LIST="$UP/root/restore-meta/runtime-packages-kept-from-backup"
    DROP_LIST="$UP/root/restore-meta/package-files-pruned-from-backup"
    : > "$KEEP_LIST"
    : > "$DROP_LIST"

    awk -v keep_pkgs="$RESTORE_KEEP_RUNTIME_PACKAGES" '
        BEGIN {
            split(keep_pkgs, a, /[ \t]+/)
            for (i in a) {
                if (a[i] != "")
                    keep[a[i]] = 1
            }
        }
        /^P:/ {
            pkg = substr($0, 3)
            selected = (pkg in keep)
            dir = ""
            next
        }
        /^F:/ {
            dir = substr($0, 3)
            next
        }
        /^R:/ && dir != "" {
            path = dir "/" substr($0, 3)
            if (path ~ /^(lib\/modules\/|etc\/modules\.d\/|lib\/apk\/|etc\/apk\/|usr\/lib\/opkg\/|etc\/opkg\/)/)
                next
            if (selected)
                print "KEEP\t" pkg "\t" path
            else
                print "DROP\t" pkg "\t" path
        }
    ' "$DB" | while IFS='	' read ACTION PKG REL; do
        [ -n "$REL" ] || continue
        case "$REL" in
            /*|../*|*/../*|..|*/..) continue ;;
        esac

        if [ "$ACTION" = "KEEP" ]; then
            echo "$PKG $REL" >> "$KEEP_LIST"
            continue
        fi

        case "$REL" in
            etc/config/*|etc/passwd|etc/group|etc/shadow|etc/gshadow|etc/profile|etc/shells|etc/hosts|etc/ethers|etc/rc.local)
                continue
                ;;
            etc/dropbear/*|etc/ssh/*|etc/ssl/*|etc/crontabs/*|etc/smartdns/*|etc/nikki/*|root/.config/nikki/*)
                continue
                ;;
            etc/*)
                case "$REL" in
                    etc/init.d/*|etc/rc.d/*|etc/uci-defaults/*) ;;
                    *) continue ;;
                esac
                ;;
        esac

        [ -e "$UP/$REL" ] || continue
        echo "$PKG $REL" >> "$DROP_LIST"
        if [ -d "$UP/$REL" ] && [ ! -L "$UP/$REL" ]; then
            rm -rf "$UP/$REL"
        else
            rm -f "$UP/$REL"
        fi
    done

    echo "已清理备份里的旧软件包文件；仅临时保留运行时：${RESTORE_KEEP_RUNTIME_PACKAGES:-<无>}"
}

prune_backup_package_files

echo "删除旧内核模块和旧包管理状态..."
rm -rf "$UP/lib/modules"
rm -rf "$UP/etc/modules.d"
rm -rf "$UP/lib/apk"
rm -rf "$UP/etc/apk"
rm -rf "$UP/usr/lib/opkg"
rm -rf "$UP/etc/opkg"

if [ -s "$DISTFEEDS_BEFORE_RESTORE" ]; then
    mkdir -p "$UP/etc/apk/repositories.d"
    cp "$DISTFEEDS_BEFORE_RESTORE" "$UP/etc/apk/repositories.d/distfeeds.list"
    echo "已保留恢复前当前系统的软件源：/etc/apk/repositories.d/distfeeds.list"
fi

if [ -s "$MYFEED_BEFORE_RESTORE" ]; then
    mkdir -p "$UP/etc/apk/repositories.d"
    cp "$MYFEED_BEFORE_RESTORE" "$UP/etc/apk/repositories.d/00-myfeed.list"
    echo "已保留恢复前当前系统的 myfeed 源：/etc/apk/repositories.d/00-myfeed.list"
elif [ -n "$RESTORE_MYFEED_REPO" ]; then
    mkdir -p "$UP/etc/apk/repositories.d"
    echo "$RESTORE_MYFEED_REPO" > "$UP/etc/apk/repositories.d/00-myfeed.list"
    echo "已写入默认 myfeed 源：/etc/apk/repositories.d/00-myfeed.list"
fi

if [ -s "$MYFEED_KEY_BEFORE_RESTORE" ]; then
    mkdir -p "$UP/etc/apk/keys"
    cp "$MYFEED_KEY_BEFORE_RESTORE" "$UP/etc/apk/keys/myfeed.pem"
    echo "已保留恢复前当前系统的 myfeed 公钥：/etc/apk/keys/myfeed.pem"
fi

schedule_package_actions() {
    [ -n "$RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES" ] || \
    [ -n "$RESTORE_INSTALL_PACKAGES" ] || \
    [ -n "$RESTORE_MYFEED_INSTALL_PACKAGES" ] || {
        echo "RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES、RESTORE_INSTALL_PACKAGES 和 RESTORE_MYFEED_INSTALL_PACKAGES 均为空，跳过重启后包操作"
        return 0
    }

    mkdir -p "$UP/etc/init.d" "$UP/etc/rc.d" "$UP/root/restore-meta"
    SCRIPT="$UP/etc/init.d/restore-package-actions"
    LOG=/root/restore-package-actions.log

    cat > "$SCRIPT" <<EOF
#!/bin/sh /etc/rc.common

START=99
LOG="$LOG"
KEEP_RUNTIME_PKGS="$RESTORE_KEEP_RUNTIME_PACKAGES"
MYFEED_KEY_URL="$RESTORE_MYFEED_KEY_URL"
REMOVE_PKGS="$RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES"
INSTALL_PKGS="$RESTORE_INSTALL_PACKAGES"
MYFEED_INSTALL_PKGS="$RESTORE_MYFEED_INSTALL_PACKAGES"

run_restore_package_actions() {
    {
        echo "== restore package actions =="
        date
        echo "remove packages: \$REMOVE_PKGS"
        echo "install packages: \$INSTALL_PKGS"
        echo "myfeed install packages: \$MYFEED_INSTALL_PKGS"
    } >> "\$LOG"

    MYFEED_FILE=/etc/apk/repositories.d/00-myfeed.list
    MYFEED_REPO=""
    MYFEED_TAGGED=0
    MYFEED_PENDING=0

    normalize_myfeed_repo() {
        [ -n "\$MYFEED_REPO" ] || return 0
        echo "\$MYFEED_REPO" > "\$MYFEED_FILE"
        if [ -f /etc/apk/world ]; then
            sed 's/@myfeed//g' /etc/apk/world > /tmp/world.no-myfeed.\$\$ && \
                mv /tmp/world.no-myfeed.\$\$ /etc/apk/world
        fi
        echo "myfeed restored to ordinary repo: \$MYFEED_REPO" >> "\$LOG"
    }

    tag_myfeed_repo() {
        [ -n "\$MYFEED_INSTALL_PKGS" ] || return 1
        [ -f "\$MYFEED_FILE" ] || {
            echo "myfeed repo file not found; install myfeed packages without tag" >> "\$LOG"
            return 1
        }

        MYFEED_REPO="\$(sed -n 's/^[[:space:]]*@myfeed[[:space:]]\{1,\}//p' "\$MYFEED_FILE" | head -n 1 | tr -d '\r')"
        if [ -z "\$MYFEED_REPO" ]; then
            MYFEED_REPO="\$(sed -n '/^[[:space:]]*#/d; /^[[:space:]]*$/d; p' "\$MYFEED_FILE" | head -n 1 | tr -d '\r')"
        fi
        [ -n "\$MYFEED_REPO" ] || {
            echo "myfeed repo is empty; install myfeed packages without tag" >> "\$LOG"
            return 1
        }

        echo "@myfeed \$MYFEED_REPO" > "\$MYFEED_FILE"
        MYFEED_TAGGED=1
        echo "myfeed temporarily tagged for install: \$MYFEED_REPO" >> "\$LOG"
        if [ -n "\$MYFEED_KEY_URL" ] && [ ! -s /etc/apk/keys/myfeed.pem ]; then
            wget -O /etc/apk/keys/myfeed.pem "\$MYFEED_KEY_URL" >> "\$LOG" 2>&1 || \
                echo "WARNING: failed to fetch myfeed key: \$MYFEED_KEY_URL" >> "\$LOG"
        fi
        return 0
    }

    start_kept_runtime() {
        for SERVICE in \$KEEP_RUNTIME_PKGS; do
            [ -x "/etc/init.d/\$SERVICE" ] || continue
            echo "starting kept runtime: \$SERVICE" >> "\$LOG"
            "/etc/init.d/\$SERVICE" start >> "\$LOG" 2>&1 || true
        done
    }

    start_kept_runtime
    sleep 5

    REMOVE_LIST=""
    for PKG in \$REMOVE_PKGS; do
        apk info -e "\$PKG" >/dev/null 2>&1 || continue
        REMOVE_LIST="\$REMOVE_LIST \$PKG"
    done

    if [ -n "\$REMOVE_LIST" ]; then
        echo "apk del:\$REMOVE_LIST" >> "\$LOG"
        apk del --force-broken-world \$REMOVE_LIST >> "\$LOG" 2>&1 || \
            echo "WARNING: apk del returned non-zero" >> "\$LOG"
    else
        echo "no matching remove packages installed" >> "\$LOG"
    fi

    INSTALL_DONE=1
    if [ -n "\$INSTALL_PKGS" ] || [ -n "\$MYFEED_INSTALL_PKGS" ]; then
        tag_myfeed_repo || MYFEED_PENDING=1

        ATTEMPT=1
        INSTALL_DONE=0
        while [ "\$ATTEMPT" -le 12 ]; do
            echo "apk update attempt \$ATTEMPT/12" >> "\$LOG"
            if apk update >> "\$LOG" 2>&1; then
                INSTALL_LIST=""
                for PKG in \$INSTALL_PKGS; do
                    apk info -e "\$PKG" >/dev/null 2>&1 && continue
                    INSTALL_LIST="\$INSTALL_LIST \$PKG"
                done
                for PKG in \$MYFEED_INSTALL_PKGS; do
                    if [ "\$MYFEED_TAGGED" = "1" ]; then
                        INSTALL_LIST="\$INSTALL_LIST \$PKG@myfeed"
                    else
                        MYFEED_PENDING=1
                    fi
                done

                if [ -z "\$INSTALL_LIST" ]; then
                    if [ "\$MYFEED_PENDING" = "1" ]; then
                        echo "myfeed packages pending; @myfeed repo is not ready" >> "\$LOG"
                    else
                        echo "install packages already present" >> "\$LOG"
                        INSTALL_DONE=1
                        break
                    fi
                else
                    echo "apk add:\$INSTALL_LIST" >> "\$LOG"
                    if apk add --force-broken-world \$INSTALL_LIST >> "\$LOG" 2>&1; then
                        if [ "\$MYFEED_PENDING" = "1" ]; then
                            echo "official packages installed; myfeed packages still pending" >> "\$LOG"
                        else
                            INSTALL_DONE=1
                            break
                        fi
                    fi
                fi
            fi

            ATTEMPT=\$((ATTEMPT + 1))
            sleep 20
        done

        normalize_myfeed_repo
    fi

    rm -rf /tmp/luci-indexcache /tmp/luci-modulecache /tmp/luci-*cache 2>/dev/null || true
    /etc/init.d/rpcd restart 2>/dev/null || true
    /etc/init.d/uwsgi restart 2>/dev/null || true
    /etc/init.d/nginx restart 2>/dev/null || true
    /etc/init.d/uhttpd restart 2>/dev/null || true

    if [ "\$INSTALL_DONE" = "1" ]; then
        echo "done; disabling restore-package-actions" >> "\$LOG"
        rm -f /etc/rc.d/S99restore-package-actions /etc/init.d/restore-package-actions
    else
        echo "WARNING: install not complete; will retry next boot" >> "\$LOG"
    fi
}

start() {
    (
        sleep 30
        run_restore_package_actions
    ) &
}
EOF
    chmod +x "$SCRIPT"
    ln -sf ../init.d/restore-package-actions "$UP/etc/rc.d/S99restore-package-actions"

    echo "$RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES" > "$UP/root/restore-meta/preinstalled-luci-remove-list"
    echo "$RESTORE_INSTALL_PACKAGES" > "$UP/root/restore-meta/package-install-list"
    echo "$RESTORE_MYFEED_INSTALL_PACKAGES" > "$UP/root/restore-meta/myfeed-package-install-list"
    echo "已安排重启后执行包操作，日志：$LOG"
}

echo "删除旧 LuCI 运行时和缓存状态..."
rm -rf "$UP/usr/lib/lua/luci"
rm -rf "$UP/usr/share/luci"
rm -rf "$UP/usr/share/ucode/luci"
rm -rf "$UP/www/luci-static"
rm -rf "$UP/www/cgi-bin/luci"
rm -rf "$UP/usr/share/rpcd/acl.d"/luci*
rm -rf "$UP/usr/libexec/rpcd/luci"

rm -f "$UP/usr/lib/lua/.wh.luci"
rm -f "$UP/usr/share/.wh.luci"
rm -f "$UP/usr/share/ucode/.wh.luci"
rm -f "$UP/www/.wh.luci-static"
rm -f "$UP/www/cgi-bin/.wh.luci"
rm -f "$UP/usr/share/rpcd/acl.d"/.wh.luci*
rm -f "$UP/usr/libexec/rpcd/.wh.luci"

rm -f "$UP/usr/lib/lua/.wh..wh..opq"
rm -f "$UP/usr/share/.wh..wh..opq"
rm -f "$UP/usr/share/ucode/.wh..wh..opq"
rm -f "$UP/www/.wh..wh..opq"
rm -f "$UP/www/cgi-bin/.wh..wh..opq"
rm -f "$UP/usr/share/rpcd/acl.d/.wh..wh..opq"
rm -f "$UP/usr/libexec/rpcd/.wh..wh..opq"

schedule_package_actions

echo "清理常见运行缓存..."
rm -f "$UP/etc/smartdns/smartdns.cache"
rm -f "$UP/etc/smartdns/data"/smartdns.cache 2>/dev/null || true
rm -rf /overlay/work /overlay/lost+found

echo "完成。系统即将重启，重启后使用新固件自带 LuCI 和包管理状态。"
sync
reboot
