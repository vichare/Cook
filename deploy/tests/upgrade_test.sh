#!/bin/sh
set -eu

root="$(CDPATH='' cd -- "$(dirname "$0")/../.." && pwd)"
temporary_directories=""
fixture=""

cleanup() {
    for directory in $temporary_directories; do
        rm -rf "$directory"
    done
}
trap cleanup EXIT

new_fixture() {
    fixture="$(mktemp -d)"
    temporary_directories="$temporary_directories $fixture"
    cp "$root/deploy/upgrade.sh" "$fixture/upgrade.sh"
    chmod +x "$fixture/upgrade.sh"
    : >"$fixture/compose.yaml"
    cat >"$fixture/.env" <<'EOF'
COOK_DOMAIN=example.invalid
EOF
    mkdir "$fixture/bin"
    cat >"$fixture/bin/flock" <<'EOF'
#!/bin/sh
exit 0
EOF
    chmod +x "$fixture/bin/flock"
}

run_expect_failure() {
    fixture="$1"
    output="$fixture/output"
    if PATH="$fixture/bin:$PATH" TEST_COMMAND_LOG="$fixture/commands" TEST_FIXTURE="$fixture" \
        "$fixture/upgrade.sh" ghcr.io/vichare/cook:v0.1.1 >"$output" 2>&1; then
        echo "expected upgrade failure" >&2
        exit 1
    fi
}

test_missing_data_directory() {
    new_fixture
    cat >"$fixture/bin/docker" <<'EOF'
#!/bin/sh
echo "docker must not run" >&2
exit 99
EOF
    chmod +x "$fixture/bin/docker"

    run_expect_failure "$fixture"

    grep -Fq "Cook data directory is missing" "$fixture/output"
    grep -Fq "sudo install -d -o 10001 -g <deploy-group> -m 2770 <deployment-directory>/data" \
        "$fixture/output"
}

test_wrong_data_owner() {
    new_fixture
    mkdir "$fixture/data"
    cat >"$fixture/bin/stat" <<'EOF'
#!/bin/sh
printf '%s\n' '1000'
EOF
    cat >"$fixture/bin/docker" <<'EOF'
#!/bin/sh
echo "docker must not run" >&2
exit 99
EOF
    chmod +x "$fixture/bin/stat" "$fixture/bin/docker"

    run_expect_failure "$fixture"

    grep -Fq "must be owned by UID 10001; found UID 1000" "$fixture/output"
    grep -Fq "sudo install -d -o 10001 -g <deploy-group> -m 2770 <deployment-directory>/data" \
        "$fixture/output"
}

test_first_deploy_rollback_stays_stopped() {
    new_fixture
    mkdir "$fixture/data"
    : >"$fixture/commands"
    cat >"$fixture/bin/stat" <<'EOF'
#!/bin/sh
printf '%s\n' '10001'
EOF
    cat >"$fixture/bin/docker" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"$TEST_COMMAND_LOG"
if [ "$*" = "compose -f compose.yaml up -d --no-deps cook" ]; then
    : >"$TEST_FIXTURE/data/cook.db"
fi
exit 0
EOF
    cat >"$fixture/bin/curl" <<'EOF'
#!/bin/sh
exit 1
EOF
    cat >"$fixture/bin/sleep" <<'EOF'
#!/bin/sh
exit 0
EOF
    chmod +x "$fixture/bin/stat" "$fixture/bin/docker" "$fixture/bin/curl" "$fixture/bin/sleep"

    run_expect_failure "$fixture"

    grep -Fq "no previous release is available; the failed container remains stopped" \
        "$fixture/output"
    if grep -q '^COOK_IMAGE=' "$fixture/.env"; then
        echo "first-deploy rollback did not restore the original environment" >&2
        exit 1
    fi
    if [ -e "$fixture/data/cook.db" ]; then
        echo "first-deploy rollback did not remove the newly created database" >&2
        exit 1
    fi
    up_count="$(
        grep -c '^compose -f compose.yaml up -d --no-deps cook$' "$fixture/commands" || true
    )"
    if [ "$up_count" -ne 1 ]; then
        echo "expected only the failed release start, found $up_count compose up calls" >&2
        exit 1
    fi
    grep -Fq "compose -f compose.yaml stop cook" "$fixture/commands"
}

test_missing_data_directory
test_wrong_data_owner
test_first_deploy_rollback_stays_stopped
echo "upgrade shell tests passed"
