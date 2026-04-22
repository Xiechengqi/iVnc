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

log "starting ivnc: /usr/local/bin/ivnc $*"
exec /usr/local/bin/ivnc "$@"
