#!/bin/sh
# Creates a consistent SQLite online backup and keeps the newest copies.
set -eu
cd "$(dirname "$0")"

if [ -z "${COOK_MAINTENANCE_LOCK_HELD:-}" ]; then
    exec 9>.maintenance.lock
    flock -w 1200 9
fi

keep="${COOK_BACKUP_KEEP:-14}"
database="data/cook.db"
if [ ! -f "$database" ]; then
    echo "no database at $database" >&2
    exit 1
fi
command -v sqlite3 >/dev/null 2>&1 || {
    echo "sqlite3 is required for consistent backups" >&2
    exit 1
}

mkdir -p backups
target="backups/cook-$(date -u +%Y%m%dT%H%M%SZ).db"
sqlite3 "$database" ".timeout 5000" ".backup '$target'"
integrity="$(sqlite3 "$target" "PRAGMA integrity_check;")"
if [ "$integrity" != "ok" ]; then
    rm -f "$target"
    echo "backup integrity check failed: $integrity" >&2
    exit 1
fi
echo "backup written to $target"

find backups -maxdepth 1 -type f -name 'cook-*.db' -print |
    sort -r |
    tail -n +"$((keep + 1))" |
    while IFS= read -r expired_backup; do
        rm -- "$expired_backup"
    done
