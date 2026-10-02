# Passive during ordinary firmware upgrades. Only the private clean-stage
# command prepared by overlay-restore changes the RAM transition environment.
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
