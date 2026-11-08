# Mark only real keep-config upgrades before the configuration archive is built.
# Ordinary backup/list/test commands and clean-overlay transitions stay passive.
overlay_restore_real_upgrade() {
    [ "${COMMAND:-}" = /lib/upgrade/do_stage2 ] || return 1
    [ "${TEST:-0}" = 0 ] && [ "${CONF_BACKUP_LIST:-0}" = 0 ] || return 1
    [ -z "${CONF_BACKUP:-}" ] && [ -z "${CONF_RESTORE:-}" ] || return 1
    overlay_restore_hook_root=${OVERLAY_RESTORE_BOOTSTRAP_ROOT:-}
    [ "$(uci -c "$overlay_restore_hook_root/etc/config" -q get overlay_restore.main)" = restore ] || return 1
    overlay_restore_hook_enabled=$(uci -c "$overlay_restore_hook_root/etc/config" -q get overlay_restore.main.upgrade_bootstrap)
    [ "${overlay_restore_hook_enabled:-1}" = 1 ] || return 1
    overlay_restore_hook_dir="$overlay_restore_hook_root/etc/overlay-restore-bootstrap"
    return 0
}

overlay_restore_mark_tool_refresh() {
    overlay_restore_real_upgrade || return 0
    overlay_restore_hook_source="$overlay_restore_hook_root/usr/libexec/overlay-restore-bootstrap"
    [ -s "$overlay_restore_hook_source" ] || overlay_restore_hook_source="$overlay_restore_hook_dir/run"
    if [ ! -s "$overlay_restore_hook_source" ] ||
       ! /bin/sh "$overlay_restore_hook_source" setup ||
       ! (cd "$overlay_restore_hook_dir" && sha256sum -c files.sha256 >/dev/null 2>&1); then
        echo 'Cannot retain complete recovery bootstrap files; firmware upgrade aborted. Reinstall overlay-restore first.' >&2
        exit 1
    fi
    if ! cat "$overlay_restore_hook_root/proc/sys/kernel/random/boot_id" > "$overlay_restore_hook_dir/refresh-pending.tmp.$$" ||
       ! mv "$overlay_restore_hook_dir/refresh-pending.tmp.$$" "$overlay_restore_hook_dir/refresh-pending"; then
        echo 'Cannot retain the recovery upgrade checkpoint; firmware upgrade aborted.' >&2
        exit 1
    fi
}

overlay_restore_keep_bootstrap_files() {
    overlay_restore_real_upgrade || return 0
    [ -n "${1:-}" ] && [ ! -L "$1" ] || exit 1
    # Append after the firmware's list builder: -u must not discard unchanged
    # package resources from /rom that the next image does not contain.
    for overlay_restore_hook_file in run init myfeed.pem runner runner-init files.sha256 refresh-pending; do
        printf '%s\n' "$overlay_restore_hook_dir/$overlay_restore_hook_file"
    done >> "$1"
    printf '%s\n' "$overlay_restore_hook_root/etc/config/overlay_restore" \
        "$overlay_restore_hook_root/etc/init.d/overlay-restore-bootstrap" \
        "$overlay_restore_hook_root/etc/rc.d/S99overlay-restore-bootstrap" \
        "$overlay_restore_hook_root/lib/upgrade/keep.d/overlay-restore-bootstrap" \
        "$overlay_restore_hook_root/lib/upgrade/zz-overlay-restore.sh" >> "$1"
    sort -u "$1" -o "$1" || exit 1
}

sysupgrade_init_conffiles="overlay_restore_mark_tool_refresh ${sysupgrade_init_conffiles:-} overlay_restore_keep_bootstrap_files"

# Only the private clean-stage command changes the RAM transition environment.
if [ "${COMMAND:-}" = '/usr/sbin/overlay-restore clean-stage /tmp/overlay-restore-stage.json' ] &&
   [ -f /tmp/overlay-restore-stage.json ]; then
    overlay_restore_stage_id="$(jsonfilter -i /tmp/overlay-restore-stage.json -e '@.id')"
    case "$overlay_restore_stage_id" in
        ''|*[!0-9a-f]*) return 1 ;;
    esac
    [ "${#overlay_restore_stage_id}" = 32 ] || return 1
    RAM_ROOT="/tmp/overlay-restore-ramfs-$overlay_restore_stage_id"
    RAMFS_COPY_BIN="$RAMFS_COPY_BIN /usr/sbin/overlay-restore /sbin/block"
    RAMFS_COPY_DATA="$RAMFS_COPY_DATA /tmp/overlay-restore-stage.json"
    # Suppress platform firmware hooks: there is no firmware image to write.
    IMAGE=
fi
