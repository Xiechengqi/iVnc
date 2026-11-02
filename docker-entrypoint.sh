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

if command -v pulseaudio >/dev/null 2>&1; then
    if ! pulseaudio --check >/dev/null 2>&1; then
        log "starting PulseAudio"
        if ! pulseaudio --start --exit-idle-time=-1; then
            fail "failed to start PulseAudio"
        fi
    fi
else
    log "PulseAudio not installed; audio capture is disabled"
fi

ivnc_args=()
cmd_args=()

# Options after the image name belong to iVnc. Keep legacy foreground
# commands (including the image's default sleep) and an explicit -- separator.
# iVnc itself defaults to /etc/ivnc.toml when no --config is supplied.
if (( $# > 0 )) && [[ "$1" != -* && "$1" != run ]]; then
    cmd_args=("$@")
else
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
fi

ivnc_pid=""
cmd_pid=""
cleanup() {
    local exit_code=$?
    trap - EXIT INT TERM
    local pid
    for pid in "$cmd_pid" "$ivnc_pid"; do
        if [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1; then
            kill "$pid" >/dev/null 2>&1 || true
        fi
    done
    for pid in "$cmd_pid" "$ivnc_pid"; do
        if [[ -n "$pid" ]]; then
            wait "$pid" >/dev/null 2>&1 || true
        fi
    done
    exit "$exit_code"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

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

while true; do
    if ! kill -0 "$ivnc_pid" >/dev/null 2>&1; then
        if wait "$ivnc_pid"; then exit_code=0; else exit_code=$?; fi
        fail "ivnc exited with code $exit_code"
    fi

    if ! kill -0 "$cmd_pid" >/dev/null 2>&1; then
        if wait "$cmd_pid"; then exit 0; else exit $?; fi
    fi

    sleep 1
done
