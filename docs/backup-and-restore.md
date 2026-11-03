# SQLite backup and restore

Do not copy the live `.db` file by itself while WAL mode is active. Use SQLite's online backup command so the database, WAL state, and page snapshot are consistent.

## Backup

On the production host, use `deploy/backup.sh`; it implements the same online backup and integrity-check sequence under the maintenance lock. The underlying commands are:

```sh
sqlite3 /data/cook.db ".timeout 5000" ".backup '/backups/cook-$(date +%F-%H%M%S).db'"
sqlite3 /backups/cook-YYYY-MM-DD-HHMMSS.db "PRAGMA integrity_check;"
```

Store backups encrypted with retention appropriate to private pantry, account, recipe, and cover-image data. Normalized cover images are SQLite BLOBs, so the online backup includes their bytes and metadata atomically; plan capacity and restore time accordingly. Periodically test restoration.

## Restore

1. Stop Cook containers to prevent writes.
2. Preserve the current database files separately.
3. Run `PRAGMA integrity_check;` on the candidate backup.
4. Restore to a new file, set the same ownership and permissions as `/data/cook.db`, and atomically replace it.
5. Remove obsolete `cook.db-wal` and `cook.db-shm` files only while the API is stopped.
6. Start the stack and run account isolation, pantry, recipe, private/public cover-image access, and recommendation smoke tests.

The exact host backup destination and retention policy are **unverified** until deployment.
