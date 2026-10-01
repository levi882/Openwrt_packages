#!/bin/sh
set -eu

# CLI entry point for the same backend used by luci-app-overlay-restore.
# The backend imports configuration through the merged filesystem and never
# deletes a mounted overlay upper/work directory.
INSPECT_ONLY=0
if [ "${1:-}" = "--inspect" ]; then
    INSPECT_ONLY=1
    shift
fi
if [ "$#" != 1 ] || [ ! -f "$1" ]; then
    echo "Usage: restore_overlay.sh [--inspect] <overlay-or-sysupgrade-backup.tar.gz>" >&2
    exit 1
fi
command -v overlay-restore >/dev/null 2>&1 || {
    echo "Install overlay-restore (or luci-app-overlay-restore) from myfeed first." >&2
    exit 1
}

RESULT="$(overlay-restore inspect "$1")" || {
    printf '%s\n' "$RESULT" >&2
    exit 1
}
printf '%s\n' "$RESULT"
[ "$INSPECT_ONLY" = 1 ] && exit 0

TASK_ID="$(printf '%s\n' "$RESULT" | jsonfilter -e '@.id')"
[ -n "$TASK_ID" ] || { echo "The backend returned no task ID." >&2; exit 1; }
echo
echo "The inspected files will replace configuration and user files."
echo "Current firmware, package state, repositories, keys, and recovery tools are preserved."
echo "Selected software packages will be restored after reboot."
echo "Credentials and the LAN address may change according to the inspected plan."
printf 'Type YES to apply task %s: ' "$TASK_ID"
read -r CONFIRM
[ "$CONFIRM" = YES ] || exit 0
overlay-restore apply "$TASK_ID" --confirm "$TASK_ID"
