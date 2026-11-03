#!/bin/sh
# Forced command for the restricted Cook CI deploy key.
set -eu
cd "$(dirname "$0")"

request="${SSH_ORIGINAL_COMMAND:-}"
case "$request" in
    "deploy "*) release="${request#deploy }" ;;
    *) echo "expected: deploy vMAJOR.MINOR.PATCH" >&2; exit 64 ;;
esac
if ! printf '%s\n' "$release" | grep -Eqx 'v[0-9]+\.[0-9]+\.[0-9]+'; then
    echo "invalid release tag" >&2
    exit 64
fi

IFS= read -r registry_user || true
IFS= read -r registry_token || true
if [ -z "$registry_user" ] || [ -z "$registry_token" ]; then
    echo "expected registry user and short-lived token on standard input" >&2
    exit 64
fi

DOCKER_CONFIG="$(mktemp -d)"
export DOCKER_CONFIG
trap 'rm -rf "$DOCKER_CONFIG"' EXIT
printf '%s\n' "$registry_token" |
    docker login ghcr.io --username "$registry_user" --password-stdin >/dev/null

mkdir -p logs
log="logs/deploy-$release-$(date -u +%Y%m%dT%H%M%SZ).log"
: >"$log"
# The quoted variables intentionally expand in the detached shell.
# shellcheck disable=SC2016
nohup sh -c './upgrade.sh "$1"; status=$?; rm -rf "$DOCKER_CONFIG"; exit "$status"' \
    upgrade "ghcr.io/vichare/cook:$release" >>"$log" 2>&1 </dev/null &
upgrade_pid=$!
trap - EXIT

tail -n +1 -f --pid="$upgrade_pid" "$log" || true
status=0
wait "$upgrade_pid" || status=$?
echo "deploy $release finished with status $status; log: $PWD/$log"
exit "$status"
