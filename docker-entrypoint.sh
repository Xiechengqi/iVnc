#!/usr/bin/env bash
set -euo pipefail

log() {
    echo "[entrypoint] $*" >&2
}

fail() {
    log "ERROR: $*"
    exit 1
}

if [[ ! -x /usr/local/bin/ivnc ]]; then
    fail "ivnc binary is missing or not executable at /usr/local/bin/ivnc"
fi

if [[ -z "${XDG_RUNTIME_DIR:-}" ]]; then
    fail "XDG_RUNTIME_DIR is not set"
fi

mkdir -p "$XDG_RUNTIME_DIR" || fail "failed to create XDG_RUNTIME_DIR: $XDG_RUNTIME_DIR"

if ! pulseaudio --check >/dev/null 2>&1; then
    log "starting PulseAudio"
    if ! pulseaudio --start --exit-idle-time=-1; then
        fail "failed to start PulseAudio"
    fi
fi

ivnc_args=()
cmd_args=()
parsing_ivnc_args=1

for arg in "$@"; do
    if [[ "$parsing_ivnc_args" -eq 1 && "$arg" == "--" ]]; then
        parsing_ivnc_args=0
        continue
    fi

    if [[ "$parsing_ivnc_args" -eq 1 ]]; then
        ivnc_args+=("$arg")
    else
        cmd_args+=("$arg")
    fi
done

if [[ "${#ivnc_args[@]}" -eq 0 ]]; then
    fail "no ivnc arguments provided to entrypoint"
fi

log "starting ivnc: /usr/local/bin/ivnc ${ivnc_args[*]}"
/usr/local/bin/ivnc "${ivnc_args[@]}" &
ivnc_pid=$!

sleep 2
if ! kill -0 "$ivnc_pid" >/dev/null 2>&1; then
    wait "$ivnc_pid" || fail "ivnc exited immediately during startup"
fi

if [[ "${#cmd_args[@]}" -eq 0 ]]; then
    log "no foreground command provided, waiting on ivnc"
    wait "$ivnc_pid"
    exit $?
fi

log "starting foreground command: ${cmd_args[*]}"
"${cmd_args[@]}" &
cmd_pid=$!

cleanup() {
    local exit_code=$?
    if kill -0 "$cmd_pid" >/dev/null 2>&1; then
        kill "$cmd_pid" >/dev/null 2>&1 || true
    fi
    if kill -0 "$ivnc_pid" >/dev/null 2>&1; then
        kill "$ivnc_pid" >/dev/null 2>&1 || true
    fi
    wait "$cmd_pid" >/dev/null 2>&1 || true
    wait "$ivnc_pid" >/dev/null 2>&1 || true
    exit "$exit_code"
}

trap cleanup INT TERM

while true; do
    if ! kill -0 "$ivnc_pid" >/dev/null 2>&1; then
        wait "$ivnc_pid"
        exit_code=$?
        fail "ivnc exited with code $exit_code"
    fi

    if ! kill -0 "$cmd_pid" >/dev/null 2>&1; then
        wait "$cmd_pid"
        exit_code=$?
        if kill -0 "$ivnc_pid" >/dev/null 2>&1; then
            kill "$ivnc_pid" >/dev/null 2>&1 || true
            wait "$ivnc_pid" >/dev/null 2>&1 || true
        fi
        exit "$exit_code"
    fi

    sleep 1
done
