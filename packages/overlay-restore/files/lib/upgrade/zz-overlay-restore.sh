# Mark only real keep-config upgrades before the configuration archive is built.
# Ordinary backup/list/test commands and clean-overlay transitions stay passive.
overlay_restore_mark_tool_refresh() {
    [ "${COMMAND:-}" = /lib/upgrade/do_stage2 ] || return 0
    [ "${TEST:-0}" = 0 ] && [ "${CONF_BACKUP_LIST:-0}" = 0 ] || return 0
    [ -z "${CONF_BACKUP:-}" ] && [ -z "${CONF_RESTORE:-}" ] || return 0
    overlay_restore_hook_root=${OVERLAY_RESTORE_BOOTSTRAP_ROOT:-}
    [ "$(uci -c "$overlay_restore_hook_root/etc/config" -q get overlay_restore.main)" = restore ] || return 0
    overlay_restore_hook_enabled=$(uci -c "$overlay_restore_hook_root/etc/config" -q get overlay_restore.main.upgrade_bootstrap)
    [ "${overlay_restore_hook_enabled:-1}" = 1 ] || return 0
    overlay_restore_hook_dir="$overlay_restore_hook_root/etc/overlay-restore-bootstrap"
    [ -d "$overlay_restore_hook_dir" ] && [ ! -L "$overlay_restore_hook_dir" ] || return 0
    cat "$overlay_restore_hook_root/proc/sys/kernel/random/boot_id" > "$overlay_restore_hook_dir/refresh-pending.tmp.$$" &&
        mv "$overlay_restore_hook_dir/refresh-pending.tmp.$$" "$overlay_restore_hook_dir/refresh-pending"
}

sysupgrade_init_conffiles="overlay_restore_mark_tool_refresh ${sysupgrade_init_conffiles:-}"

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
