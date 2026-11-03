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

previous_backup=""
cp -p .env .env.before-upgrade

rollback() {
    status=$?
    if [ "$status" -eq 0 ]; then
        return
    fi
    echo "upgrade failed; restoring previous image and database" >&2
    docker compose -f compose.yaml stop cook || true
    cp -p .env.before-upgrade .env
    if [ -n "$previous_backup" ]; then
        ./restore.sh "$previous_backup"
    fi
    docker compose -f compose.yaml up -d
    echo "rollback started; verify http://127.0.0.1:18080/health" >&2
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
    ./backup.sh
    previous_backup="$(
        find backups -maxdepth 1 -type f -name 'cook-*.db' -print |
            sort -r |
            head -n 1
    )"
fi

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
