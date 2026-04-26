#!/bin/sh
set -e

BACKUP_FILE="$1"
RESTORE_KEEP_EXTROOT="${RESTORE_KEEP_EXTROOT:-0}"
FSTAB_BEFORE_RESTORE=/tmp/restore-fstab.before-restore
DISTFEEDS_BEFORE_RESTORE=/tmp/restore-distfeeds.before-restore
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
POST_RESTORE_SRC="${POST_RESTORE_SRC:-$SCRIPT_DIR/post_restore_reinstall.sh}"

[ -f "$BACKUP_FILE" ] || {
    echo "用法：restore_overlay.sh 备份文件"
    exit 1
}

[ -f "$POST_RESTORE_SRC" ] || {
    echo "缺少 $POST_RESTORE_SRC，请将 post_restore_reinstall.sh 与 restore_overlay.sh 放在同一目录"
    exit 1
}

echo "即将恢复：$BACKUP_FILE"
echo "最低限度清理：旧 kmod/旧 apk 状态 + SmartDNS 运行缓存"
echo "配置尽量保留，软件包按恢复根包和启动代理依赖闭包恢复"
echo "恢复后会优先添加你的 myfeed，再使用当前固件官方源"
echo "输入 YES 继续："
read CONFIRM
[ "$CONFIRM" = "YES" ] || exit 0

rm -f "$FSTAB_BEFORE_RESTORE"
[ -f /etc/config/fstab ] && cp /etc/config/fstab "$FSTAB_BEFORE_RESTORE"
rm -f "$DISTFEEDS_BEFORE_RESTORE"
[ -f /etc/apk/repositories.d/distfeeds.list ] && cp /etc/apk/repositories.d/distfeeds.list "$DISTFEEDS_BEFORE_RESTORE"

echo "校验备份包..."
gzip -t "$BACKUP_FILE"
tar -tzf "$BACKUP_FILE" >/dev/null

echo "清空并恢复 overlay..."
rm -rf /overlay/*
tar -xzf "$BACKUP_FILE" -C /

UP=/overlay/upper
mkdir -p "$UP/root/restore-meta"

[ -s "$FSTAB_BEFORE_RESTORE" ] && \
    cp "$FSTAB_BEFORE_RESTORE" "$UP/root/restore-meta/fstab.current-before-restore"
[ -s "$DISTFEEDS_BEFORE_RESTORE" ] && \
    cp "$DISTFEEDS_BEFORE_RESTORE" "$UP/root/restore-meta/distfeeds.current-before-restore"

ALLOW_RE='^(luci-app-(smartdns|smartdns-lite|dockerman|lucky|vlmcsd|fakehttp|watchcat|oaf|nikki|omcproxy|rtp2httpd|samba4|webdav|easytier|bandix|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|ota|diskman|ttyd|commands|quickfile|filebrowser|filemanager|fileassistant|ramfree|autoreboot|timedreboot|aurora-config)|luci-i18n-(smartdns|smartdns-lite|fakehttp|lucky|easytier|rtp2httpd|bandix|nikki|vlmcsd|watchcat|oaf|samba4|webdav|omcproxy|dockerman|firewall|upnp|netspeedtest|speedtest|fastnet|package-manager|opkg|attendedsysupgrade|ota|diskman|ttyd|commands|quickfile|filebrowser|filemanager|fileassistant|ramfree|autoreboot|timedreboot|aurora-config)-zh-cn|luci-i18n-app-omcproxy-zh-cn|luci-theme-aurora|lucky|easytier|rtp2httpd|fakehttp|smartdns|bandix|nikki|yq|mihomo|sing-box|ip-full|firewall4|ca-bundle|ca-certificates|python3|python3-requests|tcpdump|curl|bash)$'
SKIPPED_BY_ALLOW="$UP/root/restore-meta/world.skipped-by-allowlist"
PRUNED_BY_ALLOW="$UP/root/restore-meta/packages.pruned-by-allowlist"
CURRENT_WORLD="$UP/root/restore-meta/world.from-current-rom"
RESTORE_LIST="$UP/root/apk-world.restore-list"
RESTORE_ROOTS_FILE="$UP/root/restore-meta/packages.restore-roots"
BOOTSTRAP_ROOTS_FILE="$UP/root/restore-meta/packages.bootstrap-roots"
KEEP_BACKUP_ROOTS_FILE="$UP/root/restore-meta/packages.keep-backup-roots"
KEEP_BACKUP_LUCI_FILES="$UP/root/restore-meta/packages.keep-backup-luci-files"
PRESERVED_BY_CLOSURE="$UP/root/restore-meta/packages.preserved-by-closure"
PRESERVED_BY_KEEP_BACKUP="$UP/root/restore-meta/packages.preserved-by-keep-backup"
PRESERVE_PACKAGES="$UP/root/restore-meta/packages.preserved-by-policy"
UNRESOLVED_BOOTSTRAP_DEPS="$UP/root/restore-meta/packages.bootstrap-unresolved"
UNRESOLVED_KEEP_BACKUP_DEPS="$UP/root/restore-meta/packages.keep-backup-unresolved"
LUCI_BAD_OVERLAY="$UP/root/restore-meta/luci-bad-overlay.tar.gz"
LUCI_COMMANDS_FILE="$UP/root/restore-meta/luci.commands.from-backup"
AURORA_IMAGES_BACKUP="$UP/root/restore-meta/aurora-images.from-backup.tar.gz"
: > "$SKIPPED_BY_ALLOW"
: > "$PRUNED_BY_ALLOW"
: > "$CURRENT_WORLD"
: > "$RESTORE_LIST"
: > "$RESTORE_ROOTS_FILE"
: > "$BOOTSTRAP_ROOTS_FILE"
: > "$KEEP_BACKUP_ROOTS_FILE"
: > "$KEEP_BACKUP_LUCI_FILES"
: > "$PRESERVED_BY_CLOSURE"
: > "$PRESERVED_BY_KEEP_BACKUP"
: > "$PRESERVE_PACKAGES"
: > "$UNRESOLVED_BOOTSTRAP_DEPS"
: > "$UNRESOLVED_KEEP_BACKUP_DEPS"
: > "$LUCI_COMMANDS_FILE"

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

echo "保存旧软件包清单，过滤 kernel=/kmod-*，并只保留恢复根包..."
if [ -f "$UP/etc/apk/world" ]; then
    cp "$UP/etc/apk/world" "$UP/root/restore-meta/world.from-backup"

    RESTORE_LIST_RAW=/tmp/restore-world-list.$$
    : > "$RESTORE_LIST_RAW"

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
            echo "$PKG" >> "$RESTORE_LIST_RAW"
            echo "$BASE" >> "$RESTORE_ROOTS_FILE"
        else
            echo "$BASE" >> "$SKIPPED_BY_ALLOW"
        fi
    done

    sort -u "$RESTORE_LIST_RAW" > "$RESTORE_LIST"
    rm -f "$RESTORE_LIST_RAW"

    if [ -s "$RESTORE_ROOTS_FILE" ]; then
        sort -u "$RESTORE_ROOTS_FILE" > "$RESTORE_ROOTS_FILE.tmp"
        mv "$RESTORE_ROOTS_FILE.tmp" "$RESTORE_ROOTS_FILE"
    fi

    if [ -s "$SKIPPED_BY_ALLOW" ]; then
        sort -u "$SKIPPED_BY_ALLOW" > "$SKIPPED_BY_ALLOW.tmp"
        mv "$SKIPPED_BY_ALLOW.tmp" "$SKIPPED_BY_ALLOW"
    fi
fi

normalize_dep_name() {
    echo "$1" | sed -E 's/[<>=~].*$//; s/@.*$//'
}

safe_old_pkg_name() {
    PKG="$1"
    [ -n "$PKG" ] || return 1
    case "$PKG" in
        kernel|kernel=*|kmod-*|so:*|cmd:*|*.so|*.so.*) return 1 ;;
        *[!A-Za-z0-9+_.@:-]*) return 1 ;;
    esac
    return 0
}

apk_db_has_pkg() {
    DB="$1"
    PKG="$2"
    grep -qxF "P:$PKG" "$DB"
}

apk_db_deps_for_pkg() {
    DB="$1"
    PKG="$2"
    awk -v target="$PKG" '
        $0 == "P:" target { in_pkg = 1; next }
        in_pkg && /^D:/ { print substr($0, 3); next }
        in_pkg && /^$/ { exit }
    ' "$DB"
}

detect_bootstrap_roots() {
    DB="$UP/lib/apk/db/installed"
    : > "$BOOTSTRAP_ROOTS_FILE"

    if [ -n "${RESTORE_BOOTSTRAP_ROOTS:-}" ]; then
        echo "使用 RESTORE_BOOTSTRAP_ROOTS 覆盖启动代理根包: $RESTORE_BOOTSTRAP_ROOTS"
        ROOTS="$RESTORE_BOOTSTRAP_ROOTS"
    elif [ -e "$UP/etc/config/smartdns" ] || [ -x "$UP/etc/init.d/smartdns" ] || \
         [ -e "$UP/etc/config/nikki" ] || [ -x "$UP/etc/init.d/nikki" ]; then
        ROOTS="smartdns nikki"
    else
        ROOTS=""
    fi

    [ -n "$ROOTS" ] || return 0

    for PKG in $ROOTS yq ca-bundle ca-certificates curl ip-full firewall4 mihomo sing-box; do
        BASE="$(normalize_dep_name "$PKG")"
        safe_old_pkg_name "$BASE" || continue
        echo "$BASE"
    done | sort -u > "$BOOTSTRAP_ROOTS_FILE"

    if [ -s "$BOOTSTRAP_ROOTS_FILE" ]; then
        echo "启动代理根包："
        cat "$BOOTSTRAP_ROOTS_FILE"
    fi
}

build_keep_backup_roots() {
    DB="$UP/lib/apk/db/installed"
    : > "$KEEP_BACKUP_ROOTS_FILE"

    ROOTS="${RESTORE_KEEP_BACKUP_PACKAGES:-smartdns lucky nikki luci-app-smartdns luci-app-smartdns-lite luci-app-lucky luci-i18n-lucky-zh-cn luci-app-nikki luci-i18n-nikki-zh-cn luci-i18n-nikki-zh-tw luci-i18n-nikki-ru}"
    MYFEED_KEEP_RE='^(lucky|luci-app-lucky|luci-i18n-lucky-zh-cn|easytier|luci-app-easytier|luci-i18n-easytier-zh-cn|rtp2httpd|luci-app-rtp2httpd|luci-i18n-rtp2httpd-zh-cn|fakehttp|luci-app-fakehttp|luci-i18n-fakehttp-zh-cn|smartdns|luci-app-smartdns|luci-app-smartdns-lite|bandix|luci-app-bandix|luci-i18n-bandix-zh-cn|nikki|luci-app-nikki|luci-i18n-nikki-ru|luci-i18n-nikki-zh-cn|luci-i18n-nikki-zh-tw)$'

    [ -s "$DB" ] || return 0

    {
        for PKG in $ROOTS; do
            BASE="$(normalize_dep_name "$PKG")"
            safe_old_pkg_name "$BASE" || continue
            apk_db_has_pkg "$DB" "$BASE" || continue
            echo "$BASE"
        done

        awk -v re="$MYFEED_KEEP_RE" '
            /^P:/ {
                pkg = substr($0, 3)
                if (pkg ~ re)
                    print pkg
            }
        ' "$DB"
    } | sort -u > "$KEEP_BACKUP_ROOTS_FILE"

    if [ -s "$KEEP_BACKUP_ROOTS_FILE" ]; then
        echo "保留备份运行时，不重装的软件包根："
        cat "$KEEP_BACKUP_ROOTS_FILE"
    fi
}

build_dependency_closure() {
    DB="$1"
    ROOTS_FILE="$2"
    OUT_FILE="$3"
    UNRESOLVED_FILE="$4"

    : > "$OUT_FILE"
    : > "$UNRESOLVED_FILE"
    [ -s "$DB" ] || return 0
    [ -s "$ROOTS_FILE" ] || return 0

    TODO=/tmp/restore-closure-todo.$$
    NEXT=/tmp/restore-closure-next.$$
    DONE=/tmp/restore-closure-done.$$
    : > "$DONE"
    sort -u "$ROOTS_FILE" > "$TODO"

    while [ -s "$TODO" ]; do
        : > "$NEXT"

        while IFS= read -r PKG; do
            [ -n "$PKG" ] || continue
            safe_old_pkg_name "$PKG" || continue
            grep -qxF "$PKG" "$DONE" 2>/dev/null && continue

            if ! apk_db_has_pkg "$DB" "$PKG"; then
                echo "$PKG" >> "$UNRESOLVED_FILE"
                continue
            fi

            echo "$PKG" >> "$DONE"

            apk_db_deps_for_pkg "$DB" "$PKG" | tr ' ' '\n' | while IFS= read -r DEP; do
                DEP_BASE="$(normalize_dep_name "$DEP")"
                safe_old_pkg_name "$DEP_BASE" || continue
                grep -qxF "$DEP_BASE" "$DONE" 2>/dev/null && continue
                echo "$DEP_BASE" >> "$NEXT"
            done
        done < "$TODO"

        sort -u "$NEXT" > "$TODO"
    done

    sort -u "$DONE" > "$OUT_FILE"
    sort -u "$UNRESOLVED_FILE" > "$UNRESOLVED_FILE.tmp"
    mv "$UNRESOLVED_FILE.tmp" "$UNRESOLVED_FILE"
    rm -f "$TODO" "$NEXT" "$DONE"
}

build_preserve_policy() {
    DB="$UP/lib/apk/db/installed"

    detect_bootstrap_roots
    build_keep_backup_roots
    build_dependency_closure "$DB" "$BOOTSTRAP_ROOTS_FILE" "$PRESERVED_BY_CLOSURE" "$UNRESOLVED_BOOTSTRAP_DEPS"
    build_dependency_closure "$DB" "$KEEP_BACKUP_ROOTS_FILE" "$PRESERVED_BY_KEEP_BACKUP" "$UNRESOLVED_KEEP_BACKUP_DEPS"

    {
        cat "$CURRENT_WORLD" 2>/dev/null
        cat "$RESTORE_ROOTS_FILE" 2>/dev/null
        cat "$PRESERVED_BY_CLOSURE" 2>/dev/null
        cat "$PRESERVED_BY_KEEP_BACKUP" 2>/dev/null
    } | sed '/^$/d' | sort -u > "$PRESERVE_PACKAGES"

    if [ -s "$PRESERVED_BY_CLOSURE" ]; then
        echo "按启动代理依赖闭包临时保留的软件包："
        cat "$PRESERVED_BY_CLOSURE"
    fi

    if [ -s "$PRESERVED_BY_KEEP_BACKUP" ]; then
        echo "按备份运行时策略保留、不重装的软件包闭包："
        cat "$PRESERVED_BY_KEEP_BACKUP"
    fi

    if [ -s "$UNRESOLVED_BOOTSTRAP_DEPS" ]; then
        echo "启动代理闭包中源自备份 apk db 但无法解析的依赖/虚拟包："
        cat "$UNRESOLVED_BOOTSTRAP_DEPS"
    fi

    if [ -s "$UNRESOLVED_KEEP_BACKUP_DEPS" ]; then
        echo "备份运行时保留闭包中无法解析的依赖/虚拟包："
        cat "$UNRESOLVED_KEEP_BACKUP_DEPS"
    fi
}

build_preserve_policy

record_keep_backup_luci_files() {
    DB="$UP/lib/apk/db/installed"
    [ -s "$DB" ] || return 0
    [ -s "$KEEP_BACKUP_ROOTS_FILE" ] || return 0

    awk '
        FNR == NR {
            keep[$0] = 1
            next
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
        /^R:/ && selected && dir != "" {
            path = dir "/" substr($0, 3)
            if (path ~ /^(usr\/lib\/lua\/luci\/|usr\/share\/luci\/|www\/luci-static\/|usr\/share\/rpcd\/acl\.d\/luci-|usr\/libexec\/rpcd\/luci)/)
                print path
        }
    ' "$KEEP_BACKUP_ROOTS_FILE" "$DB" | sort -u > "$KEEP_BACKUP_LUCI_FILES"

    if [ -s "$KEEP_BACKUP_LUCI_FILES" ]; then
        echo "将从备份恢复的保留包 LuCI 文件："
        cat "$KEEP_BACKUP_LUCI_FILES"
    fi
}

record_keep_backup_luci_files

preserve_keep_backup_apk_state() {
    OLD_DB="$UP/lib/apk/db/installed"
    ROM_DB=/rom/lib/apk/db/installed
    KEEP="$PRESERVED_BY_KEEP_BACKUP"
    OUT="$UP/root/restore-meta/apk.installed.keep-backup-merged"
    OLD_SELECTED="$UP/root/restore-meta/apk.installed.keep-backup-only"
    WORLD_KEEP="$UP/root/restore-meta/world.keep-backup"

    rm -f "$OUT" "$OLD_SELECTED" "$WORLD_KEEP"

    [ -s "$OLD_DB" ] || return 0
    [ -s "$KEEP" ] || return 0

    awk -v keep_file="$KEEP" '
        BEGIN {
            while ((getline line < keep_file) > 0)
                keep[line] = 1
            close(keep_file)
            RS = ""
            ORS = "\n\n"
        }
        {
            pkg = ""
            n = split($0, lines, "\n")
            for (i = 1; i <= n; i++) {
                if (lines[i] ~ /^P:/) {
                    pkg = substr(lines[i], 3)
                    break
                }
            }
            if (pkg in keep)
                print
        }
    ' "$OLD_DB" > "$OLD_SELECTED"

    [ -s "$OLD_SELECTED" ] || {
        rm -f "$OLD_SELECTED"
        return 0
    }

    if [ -s "$ROM_DB" ]; then
        awk -v keep_file="$KEEP" '
            BEGIN {
                while ((getline line < keep_file) > 0)
                    drop[line] = 1
                close(keep_file)
                RS = ""
                ORS = "\n\n"
            }
            {
                pkg = ""
                n = split($0, lines, "\n")
                for (i = 1; i <= n; i++) {
                    if (lines[i] ~ /^P:/) {
                        pkg = substr(lines[i], 3)
                        break
                    }
                }
                if (!(pkg in drop))
                    print
            }
        ' "$ROM_DB" > "$OUT"
    else
        : > "$OUT"
    fi

    cat "$OLD_SELECTED" >> "$OUT"

    if [ -s "$KEEP_BACKUP_ROOTS_FILE" ]; then
        sort -u "$KEEP_BACKUP_ROOTS_FILE" > "$WORLD_KEEP"
    fi

    echo "已合并保留包 apk 状态，恢复后这些包不会被重装覆盖："
    cat "$KEEP"
}

preserve_keep_backup_apk_state

preserve_luci_command_sections() {
    SRC="$UP/etc/config/luci"
    [ -f "$SRC" ] || return 0

    TMP="$LUCI_COMMANDS_FILE.tmp"
    awk '
        /^[ \t]*config[ \t]+/ {
            keep = ($2 == "command" || $2 == "'\''command'\''" || $2 == "\"command\"")
        }
        keep {
            print
        }
    ' "$SRC" > "$TMP" || {
        rm -f "$TMP"
        return 0
    }

    if [ -s "$TMP" ]; then
        mv "$TMP" "$LUCI_COMMANDS_FILE"
        echo "已保存 LuCI 自定义命令配置段"
    else
        rm -f "$TMP"
    fi
}

preserve_luci_command_sections

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

    if [ -d "$UP/www/luci-static/aurora/images" ]; then
        tar -czf "$AURORA_IMAGES_BACKUP" \
            -C "$UP" \
            www/luci-static/aurora/images \
            2>/dev/null || true
    fi

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

    if [ -s "$AURORA_IMAGES_BACKUP" ]; then
        tar -xzf "$AURORA_IMAGES_BACKUP" -C "$UP" 2>/dev/null || true
        echo "已保留 Aurora 主题上传资源"
    fi

    if [ -s "$KEEP_BACKUP_LUCI_FILES" ] && [ -s "$LUCI_BAD_OVERLAY" ]; then
        while IFS= read -r REL; do
            [ -n "$REL" ] || continue
            case "$REL" in
                /*|../*|*/../*|..|*/..) continue ;;
                www/cgi-bin/luci) continue ;;
            esac
            tar -xzf "$LUCI_BAD_OVERLAY" -C "$UP" "$REL" 2>/dev/null || true
        done < "$KEEP_BACKUP_LUCI_FILES"
        echo "已恢复备份保留包自己的 LuCI 页面/ACL 文件"
    fi
}

sanitize_luci_overlay

prune_non_allowlist_overlay_files() {
    DB="$UP/lib/apk/db/installed"
    [ -f "$DB" ] || return 0

    if [ ! -s "$PRESERVE_PACKAGES" ]; then
        echo "未生成保留包策略，跳过旧包文件清理"
        return 0
    fi

    echo "清理旧 overlay 中非当前固件、非恢复根包、非启动代理闭包的软件包文件..."

    awk '
        FNR == NR {
            keep[$0] = 1
            next
        }
        /^P:/ {
            pkg = substr($0, 3)
            if (!(pkg in keep)) {
                print pkg
            }
        }
    ' "$PRESERVE_PACKAGES" "$DB" | sort -u > "$PRUNED_BY_ALLOW"

    awk '
        FNR == NR {
            keep[$0] = 1
            next
        }
        /^P:/ {
            pkg = substr($0, 3)
            prune = !(pkg in keep)
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
    ' "$PRESERVE_PACKAGES" "$DB" | sort -u | while IFS= read -r REL; do
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

echo "跳过备份中的自定义软件源，恢复时按当前固件规则重建..."

sanitize_extroot_fstab() {
    FSTAB="$UP/etc/config/fstab"

    if [ "$RESTORE_KEEP_EXTROOT" = "1" ]; then
        echo "RESTORE_KEEP_EXTROOT=1：保留备份中的 extroot 挂载配置"
        return 0
    fi

    if [ -s "$UP/root/restore-meta/fstab.current-before-restore" ]; then
        mkdir -p "$UP/etc/config"
        [ -f "$FSTAB" ] && cp "$FSTAB" "$UP/root/restore-meta/fstab.from-backup"

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
            ' "$FSTAB" > "$FSTAB_TMP"
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
        ' "$UP/root/restore-meta/fstab.current-before-restore" >> "$FSTAB_TMP"

        mv "$FSTAB_TMP" "$FSTAB"
        echo "使用备份中的普通挂载点，并保留恢复前当前系统的 NVMe /overlay extroot"
        return 0
    fi

    [ -f "$FSTAB" ] || return 0

    cp "$FSTAB" "$UP/root/restore-meta/fstab.before-extroot-sanitize"

    awk '
        function flush(    i, line) {
            if (n == 0) {
                return
            }

            if (is_extroot) {
                for (i = 1; i <= n; i++) {
                    line = lines[i]
                    if (line ~ /^[ \t]*option[ \t]+enabled[ \t]+/) {
                        continue
                    }
                    if (line ~ /^[ \t]*option[ \t]+enabled_fsck[ \t]+/) {
                        continue
                    }
                    print line
                }
                print "\toption enabled '\''0'\''"
                print "\toption enabled_fsck '\''0'\''"
            } else {
                for (i = 1; i <= n; i++) {
                    print lines[i]
                }
            }

            n = 0
            is_extroot = 0
        }

        /^[ \t]*config[ \t]+/ {
            flush()
        }

        {
            lines[++n] = $0
            if ($0 ~ /^[ \t]*option[ \t]+target[ \t]+/) {
                target = $0
                sub(/^[ \t]*option[ \t]+target[ \t]+/, "", target)
                gsub(/\047/, "", target)
                gsub(/"/, "", target)
                gsub(/^[ \t]+|[ \t]+$/, "", target)
                if (target == "/overlay" || target == "/") {
                    is_extroot = 1
                }
            }
        }

        END {
            flush()
        }
    ' "$FSTAB" > "$FSTAB.tmp" && mv "$FSTAB.tmp" "$FSTAB"

    echo "已禁用备份中的 extroot 自动挂载（target /overlay 或 /），避免旧外部 overlay 污染恢复"
}

sanitize_extroot_fstab

enable_docker_data_mount() {
    FSTAB="$UP/etc/config/fstab"
    DOCKERD="$UP/etc/config/dockerd"

    [ -f "$FSTAB" ] || return 0
    [ -f "$DOCKERD" ] || return 0

    DATA_ROOT="$(awk '
        $1 == "option" && $2 == "data_root" {
            v = $0
            sub(/^[ \t]*option[ \t]+data_root[ \t]+/, "", v)
            gsub(/\047/, "", v)
            gsub(/"/, "", v)
            gsub(/^[ \t]+|[ \t]+$/, "", v)
            print v
            exit
        }
    ' "$DOCKERD")"

    [ -n "$DATA_ROOT" ] || return 0
    case "$DATA_ROOT" in
        /mnt/*/*|/mnt/*) ;;
        *) return 0 ;;
    esac

    cp "$FSTAB" "$UP/root/restore-meta/fstab.before-docker-mount-enable"

    awk -v data_root="$DATA_ROOT" '
        function flush(    i, should_enable, line) {
            if (n == 0) return

            should_enable = 0
            if (target != "" && target != "/" && target != "/overlay") {
                if (data_root == target || index(data_root, target "/") == 1) {
                    should_enable = 1
                    changed = 1
                }
            }

            if (should_enable) {
                for (i = 1; i <= n; i++) {
                    line = lines[i]
                    if (line ~ /^[ \t]*option[ \t]+enabled[ \t]+/) continue
                    print line
                }
                print "\toption enabled '\''1'\''"
            } else {
                for (i = 1; i <= n; i++) print lines[i]
            }

            n = 0
            target = ""
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
            }
        }
        END {
            flush()
            exit changed ? 0 : 2
        }
    ' "$FSTAB" > "$FSTAB.tmp"
    RC=$?

    if [ "$RC" = "0" ]; then
        mv "$FSTAB.tmp" "$FSTAB"
        echo "已启用 Docker data_root 对应的数据盘挂载: $DATA_ROOT"
    else
        rm -f "$FSTAB.tmp"
        echo "未找到 Docker data_root 对应的 fstab 挂载项: $DATA_ROOT"
    fi
}

enable_docker_data_mount

echo "最低限度删除旧内核模块和旧 apk 状态..."
rm -rf "$UP/lib/modules"
rm -rf "$UP/lib/apk"
rm -rf "$UP/etc/apk"
rm -rf "$UP/etc/modules.d"

if [ -s "$UP/root/restore-meta/apk.installed.keep-backup-merged" ]; then
    mkdir -p "$UP/lib/apk/db"
    cp "$UP/root/restore-meta/apk.installed.keep-backup-merged" "$UP/lib/apk/db/installed"
    echo "已恢复保留包 apk installed 状态，避免 Nikki 等保留包被视为未安装"
fi

if [ -s "$UP/root/restore-meta/distfeeds.current-before-restore" ] && \
   grep -q 'mirrors.cloud.tencent.com/openwrt' "$UP/root/restore-meta/distfeeds.current-before-restore" && \
   grep -q 'opkg.cooluc.com/openwrt-' "$UP/root/restore-meta/distfeeds.current-before-restore" && \
   grep -q 'core3.cooluc.com' "$UP/root/restore-meta/distfeeds.current-before-restore" && \
   ! grep -q 'downloads.openwrt.org' "$UP/root/restore-meta/distfeeds.current-before-restore" && \
   ! grep -q '/targets/.*/packages/packages\.adb' "$UP/root/restore-meta/distfeeds.current-before-restore"; then
    mkdir -p "$UP/etc/apk/repositories.d"
    cp "$UP/root/restore-meta/distfeeds.current-before-restore" "$UP/etc/apk/repositories.d/distfeeds.list"
    echo "已把恢复前干净 distfeeds.list 写回 overlay，重启后立即使用正确软件源"
else
    echo "恢复前 distfeeds.list 不存在或未通过干净源校验，重启后由 post_restore_reinstall.sh 重建"
fi

echo "清理 SmartDNS 运行缓存..."
rm -f "$UP/etc/smartdns/smartdns.cache"
rm -f "$UP/etc/smartdns/data"/smartdns.cache 2>/dev/null || true

rm -rf /overlay/work /overlay/lost+found

cp "$POST_RESTORE_SRC" "$UP/root/post_restore_reinstall.sh"

chmod +x "$UP/root/post_restore_reinstall.sh"

echo "第一阶段完成，系统即将重启。"
echo "重启后 SSH 执行："
echo "/root/post_restore_reinstall.sh"

sync
reboot
