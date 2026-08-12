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
    echo "/dev/net/tun is required for miao global and process proxy modes" >&2
    exit 1
fi

if docker container inspect "${NAME}" >/dev/null 2>&1; then
    echo "container ${NAME} already exists; remove or rename it before continuing" >&2
    exit 1
fi

mkdir -p "${ROOT_DIR}/ivnc-data" "${ROOT_DIR}/miao-data"

exec docker run -itd \
    -p "${PORT}:8008" \
    --cap-add NET_ADMIN \
    --device /dev/net/tun \
    -v /etc/hosts:/etc/hosts:ro \
    -v "${ROOT_DIR}/ivnc-data:/root/.ivnc" \
    -v "${ROOT_DIR}/miao-data:/root/.miao" \
    --restart unless-stopped \
    --name "${NAME}" \
    "${IMAGE}"
