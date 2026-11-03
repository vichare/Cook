#!/bin/sh
# Pulls and deploys a release image with SQLite backup and automatic rollback.
set -eu
cd "$(dirname "$0")"

exec 9>.maintenance.lock
flock -w 1200 9
export COOK_MAINTENANCE_LOCK_HELD=1

image_reference="${1:-}"
case "$image_reference" in
    ghcr.io/vichare/cook:v[0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "usage: ./upgrade.sh ghcr.io/vichare/cook:vMAJOR.MINOR.PATCH" >&2; exit 64 ;;
esac
if ! printf '%s\n' "$image_reference" |
    grep -Eqx 'ghcr\.io/vichare/cook:v[0-9]+\.[0-9]+\.[0-9]+'; then
    echo "invalid Cook image reference" >&2
    exit 64
fi

data_directory="data"
bootstrap_hint="sudo install -d -o 10001 -g <deploy-group> -m 2770 <deployment-directory>/data"
if [ ! -d "$data_directory" ]; then
    echo "Cook data directory is missing; create it for container UID 10001 before deploying." >&2
    echo "Example: $bootstrap_hint" >&2
    exit 1
fi
data_owner_uid="$(stat -c '%u' "$data_directory")"
if [ "$data_owner_uid" != "10001" ]; then
    echo "Cook data directory must be owned by UID 10001; found UID $data_owner_uid." >&2
    echo "Fix it during host bootstrap, for example: $bootstrap_hint" >&2
    exit 1
fi
if [ ! -r "$data_directory" ] || [ ! -w "$data_directory" ] || [ ! -x "$data_directory" ]; then
    echo "The deployment user must be able to read, write, and traverse the Cook data directory." >&2
    echo "Configure a shared deployment group during host bootstrap, for example: $bootstrap_hint" >&2
    exit 1
fi

mkdir -p backups logs

previous_backup=""
previous_image="$(sed -n 's/^COOK_IMAGE=//p' .env | tail -n 1)"
database_existed=0
new_release_started=0
cp -p .env .env.before-upgrade

rollback() {
    status=$?
    if [ "$status" -eq 0 ]; then
        return
    fi
    echo "upgrade failed; restoring previous configuration and database" >&2
    if [ "$new_release_started" -eq 1 ]; then
        docker compose -f compose.yaml stop cook || true
    fi
    cp -p .env.before-upgrade .env
    rm -f .env.before-upgrade
    if [ -n "$previous_backup" ]; then
        ./restore.sh "$previous_backup"
    elif [ "$database_existed" -eq 0 ]; then
        rm -f data/cook.db data/cook.db-wal data/cook.db-shm
    fi
    if [ -n "$previous_image" ]; then
        docker compose -f compose.yaml up -d --no-deps cook
        echo "rollback started for previous release $previous_image; verify the health endpoint" >&2
    else
        echo "no previous release is available; the failed container remains stopped" >&2
    fi
}
trap rollback EXIT

if grep -q '^COOK_IMAGE=' .env; then
    sed -i "s|^COOK_IMAGE=.*|COOK_IMAGE=$image_reference|" .env
else
    printf '\nCOOK_IMAGE=%s\n' "$image_reference" >> .env
fi

if ! docker compose -f compose.yaml pull cook; then
    echo "image pull failed; using a matching cached image only if available" >&2
    docker image inspect "$image_reference" >/dev/null
fi

if [ -f data/cook.db ]; then
    database_existed=1
    ./backup.sh
    previous_backup="$(
        find backups -maxdepth 1 -type f -name 'cook-*.db' -print |
            sort -r |
            head -n 1
    )"
fi

new_release_started=1
docker compose -f compose.yaml up -d --no-deps cook

attempt=0
until curl -fsS -o /dev/null http://127.0.0.1:18080/health; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then
        docker compose -f compose.yaml logs --tail=100 cook >&2 || true
        echo "new Cook release is not healthy after 60 seconds" >&2
        exit 1
    fi
    sleep 2
done

trap - EXIT
rm -f .env.before-upgrade
docker image prune -f
echo "upgrade complete: $image_reference"
