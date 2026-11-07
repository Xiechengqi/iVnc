#!/usr/bin/env bash
set -euo pipefail

PORT="${PORT:-8008}"
NAME="${NAME:-ivnc}"
IMAGE="${IMAGE:-ghcr.io/xiechengqi/ivnc:latest}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v docker >/dev/null 2>&1; then
    echo "docker is required but was not found in PATH" >&2
    exit 1
fi

if [[ ! "${PORT}" =~ ^[0-9]+$ ]] || ((10#${PORT} < 1 || 10#${PORT} > 65535)); then
    echo "PORT must be an integer between 1 and 65535" >&2
    exit 1
fi

if [[ ! "${NAME}" =~ ^[a-zA-Z0-9][a-zA-Z0-9_.-]*$ ]]; then
    echo "NAME must be a valid Docker container name" >&2
    exit 1
fi

if [[ -z "${IMAGE}" ]]; then
    echo "IMAGE must not be empty" >&2
    exit 1
fi

if [[ ! -c /dev/net/tun ]]; then
    echo "/dev/net/tun is required for desktop application networking and miao proxy modes" >&2
    exit 1
fi

if docker container inspect "${NAME}" >/dev/null 2>&1; then
    echo "container ${NAME} already exists; remove or rename it before continuing" >&2
    exit 1
fi

# Hosts that forbid unprivileged user namespaces cannot start desktop applications.
userns_hint() {
    echo "warning: $1; desktop applications will fail to start." >&2
    echo "         $2" >&2
}
if [[ "$(cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns 2>/dev/null)" == "1" ]]; then
    userns_hint "kernel.apparmor_restrict_unprivileged_userns=1 restricts user namespaces in containers" \
        "Run 'sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0' (persist it in /etc/sysctl.d) or load an AppArmor profile that allows userns."
fi
if [[ "$(cat /proc/sys/kernel/unprivileged_userns_clone 2>/dev/null)" == "0" ]]; then
    userns_hint "kernel.unprivileged_userns_clone=0" "Run 'sudo sysctl -w kernel.unprivileged_userns_clone=1'."
fi
if [[ "$(cat /proc/sys/user/max_user_namespaces 2>/dev/null)" == "0" ]]; then
    userns_hint "user.max_user_namespaces=0" "Run 'sudo sysctl -w user.max_user_namespaces=15000'."
fi

mkdir -p "${ROOT_DIR}/ivnc-data" "${ROOT_DIR}/miao-data"

# Each desktop application runs in its own bubblewrap sandbox with nested
# user/mount/PID/network namespaces and a private /proc. Docker's default seccomp
# and AppArmor profiles forbid creating namespaces and mounting, and its masked
# /proc paths stop the sandbox from mounting /proc. The image's iVnc applies a
# seccomp filter to itself and everything it starts, and a stricter one inside
# strict sandboxes, so no profile file is needed; SECCOMP_PROFILE can still name
# one. Container root, which is host root without userns-remap, can write
# /proc/sys: trust what runs in the container outside strict sandboxes.
SECCOMP_OPT="seccomp=${SECCOMP_PROFILE:-unconfined}"
exec docker run -itd \
    -p "${PORT}:8008" \
    --cap-add NET_ADMIN \
    --security-opt "${SECCOMP_OPT}" \
    --security-opt apparmor=unconfined \
    --security-opt systempaths=unconfined \
    --device /dev/net/tun \
    -v /etc/hosts:/etc/hosts:ro \
    -v "${ROOT_DIR}/ivnc-data:/root/.ivnc" \
    -v "${ROOT_DIR}/miao-data:/root/.miao" \
    --restart unless-stopped \
    --name "${NAME}" \
    "${IMAGE}" "$@"
