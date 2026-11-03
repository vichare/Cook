#!/bin/sh
set -eu

cook-api &
api_pid=$!

terminate() {
    kill -TERM "$api_pid" 2>/dev/null || true
    wait "$api_pid" 2>/dev/null || true
}
trap terminate INT TERM EXIT

attempt=0
until curl -fsS -o /dev/null http://127.0.0.1:8081/health; do
    attempt=$((attempt + 1))
    if ! kill -0 "$api_pid" 2>/dev/null || [ "$attempt" -ge 30 ]; then
        echo "Cook API failed to become healthy" >&2
        exit 1
    fi
    sleep 1
done

nginx -g 'daemon off;' &
nginx_pid=$!
wait "$nginx_pid"
