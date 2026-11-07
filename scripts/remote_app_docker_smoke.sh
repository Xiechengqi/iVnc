#!/usr/bin/env bash
# Run the remote sharing suites inside an existing iVnc runtime image.
# Build the current tree first: docker build -t ivnc:remote-review .
# IVNC_TEST_IMAGE=ivnc:remote-review bash scripts/remote_app_docker_smoke.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${IVNC_TEST_IMAGE:-ivnc:remote-review}"
docker info >/dev/null
docker image inspect "$IMAGE" >/dev/null
exec docker run --rm --init \
  --security-opt seccomp=unconfined --security-opt apparmor=unconfined \
  --security-opt systempaths=unconfined --device /dev/net/tun \
  --shm-size=1g --entrypoint bash \
  -v "$ROOT:/smoke:ro" -w /smoke \
  -e IVNC_BIN=/usr/local/bin/ivnc "$IMAGE" -ceu '
    apt-get update >/dev/null
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      gtk-3-examples python3 nodejs node-ws jq >/dev/null
    bash scripts/remote_app_smoke.sh
    REMOTE_SMOKE_TLS=1 bash scripts/remote_app_smoke.sh
    bash scripts/remote_hardening_smoke.sh
    python3 scripts/remote_isolation_smoke.py
  '
