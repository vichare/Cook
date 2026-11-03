#!/bin/sh
set -eu

event_pipe="${TMPDIR:-/tmp}/cook-supervisor.$$"
mkfifo "$event_pipe"
exec 3<>"$event_pipe"
rm -f "$event_pipe"

api_pid=""
nginx_pid=""

run_supervised() {
    process_name="$1"
    shift
    "$@" &
    child_pid=$!
    termination_requested=0

    # Invoked indirectly by the signal trap.
    # shellcheck disable=SC2329
    forward_signal() {
        termination_requested=1
        kill -TERM "$child_pid" 2>/dev/null || true
    }
    trap forward_signal INT TERM

    child_status=0
    wait "$child_pid" || child_status=$?
    if [ "$termination_requested" -eq 1 ]; then
        wait "$child_pid" 2>/dev/null || true
    fi
    trap - INT TERM
    printf '%s %s\n' "$process_name" "$child_status" >&3
    exit "$child_status"
}

# Invoked indirectly by the exit and signal traps.
# shellcheck disable=SC2329
cleanup() {
    trap - INT TERM EXIT
    if [ -n "$nginx_pid" ]; then
        kill -TERM "$nginx_pid" 2>/dev/null || true
    fi
    if [ -n "$api_pid" ]; then
        kill -TERM "$api_pid" 2>/dev/null || true
    fi
    if [ -n "$nginx_pid" ]; then
        wait "$nginx_pid" 2>/dev/null || true
    fi
    if [ -n "$api_pid" ]; then
        wait "$api_pid" 2>/dev/null || true
    fi
    exec 3>&-
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

run_supervised api cook-api &
api_pid=$!

attempt=0
while :; do
    health_response=""
    if health_response="$(curl -fsS http://127.0.0.1:8081/health)" &&
        [ "$health_response" = "ok" ]; then
        break
    fi
    attempt=$((attempt + 1))
    if ! kill -0 "$api_pid" 2>/dev/null || [ "$attempt" -ge 30 ]; then
        echo "Cook API failed to become healthy" >&2
        exit 1
    fi
    sleep 1
done

run_supervised nginx nginx -g 'daemon off;' &
nginx_pid=$!

exited_process=""
exit_status=""
IFS=' ' read -r exited_process exit_status <&3

case "$exited_process" in
    api)
        echo "Cook API exited with status $exit_status; stopping nginx" >&2
        kill -TERM "$nginx_pid" 2>/dev/null || true
        wait "$nginx_pid" 2>/dev/null || true
        if [ "$exit_status" -eq 0 ]; then
            exit_status=1
        fi
        ;;
    nginx)
        echo "nginx exited with status $exit_status; stopping Cook API" >&2
        kill -TERM "$api_pid" 2>/dev/null || true
        wait "$api_pid" 2>/dev/null || true
        if [ "$exit_status" -eq 0 ]; then
            exit_status=1
        fi
        ;;
    *)
        echo "unknown supervised process event: $exited_process" >&2
        exit_status=1
        ;;
esac

exit "$exit_status"
