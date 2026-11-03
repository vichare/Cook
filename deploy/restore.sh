#!/bin/sh
# Restores one verified backup while Cook is stopped.
set -eu
cd "$(dirname "$0")"

backup="${1:-}"
if [ -z "$backup" ] || [ ! -f "$backup" ]; then
    echo "usage: ./restore.sh backups/cook-YYYYMMDDTHHMMSSZ.db" >&2
    exit 64
fi
command -v sqlite3 >/dev/null 2>&1 || {
    echo "sqlite3 is required for restore verification" >&2
    exit 1
}
if [ "$(sqlite3 "$backup" "PRAGMA integrity_check;")" != "ok" ]; then
    echo "backup integrity check failed" >&2
    exit 1
fi
if [ -n "$(docker compose -f compose.yaml ps --status running --quiet cook)" ]; then
    echo "stop Cook before restoring a database" >&2
    exit 1
fi

mkdir -p data
cp "$backup" data/cook.db.restore
mv -f data/cook.db.restore data/cook.db
rm -f data/cook.db-wal data/cook.db-shm
echo "restored $backup to data/cook.db"
