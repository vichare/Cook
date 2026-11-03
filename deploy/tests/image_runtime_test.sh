#!/bin/sh
set -eu

root="$(CDPATH='' cd -- "$(dirname "$0")/../.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT

grep -Fq "location = /health {" "$root/deploy/image-nginx.conf"
grep -Fq "proxy_pass http://127.0.0.1:8081/health;" "$root/deploy/image-nginx.conf"
grep -Fq "client_max_body_size 9m;" "$root/deploy/image-nginx.conf"
grep -Fq "client_max_body_size 9m;" "$root/deploy/container-nginx.conf"
grep -Fq "client_max_body_size 9m;" "$root/deploy/nginx/cook.vichare.net.conf"
grep -Fq "FROM rust:1.89-bookworm AS api-build" "$root/Dockerfile"
grep -Fq "FROM rust:1.89-bookworm AS build" "$root/apps/api/Dockerfile"
# The literal command substitution is part of the Compose healthcheck.
# shellcheck disable=SC2016
grep -Fq 'test: ["CMD-SHELL", "[ \"$(curl -fsS http://127.0.0.1:8080/health)\" = ok ]"]' \
    "$root/deploy/compose.yaml"

cp "$root/deploy/image-entrypoint.sh" "$fixture/entrypoint.sh"
chmod +x "$fixture/entrypoint.sh"
mkdir "$fixture/bin"

cat >"$fixture/bin/cook-api" <<'EOF'
#!/bin/sh
sleep 1
exit 0
EOF
cat >"$fixture/bin/curl" <<'EOF'
#!/bin/sh
printf '%s\n' 'ok'
EOF
cat >"$fixture/bin/nginx" <<'EOF'
#!/bin/sh
trap 'printf "%s\n" terminated >"$TEST_NGINX_MARKER"; exit 0' INT TERM
while :; do
    sleep 1
done
EOF
chmod +x "$fixture/bin/cook-api" "$fixture/bin/curl" "$fixture/bin/nginx"

status=0
PATH="$fixture/bin:$PATH" TEST_NGINX_MARKER="$fixture/nginx-terminated" \
    timeout 10 "$fixture/entrypoint.sh" >"$fixture/output" 2>&1 || status=$?
if [ "$status" -eq 0 ] || [ "$status" -eq 124 ]; then
    echo "entrypoint did not fail promptly after the API exited; status=$status" >&2
    cat "$fixture/output" >&2
    exit 1
fi
grep -Fq "Cook API exited with status 0; stopping nginx" "$fixture/output"
grep -Fq "terminated" "$fixture/nginx-terminated"

echo "image runtime shell tests passed"
