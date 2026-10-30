#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
WEB_DIR="$ROOT/web/ivnc"
DIST_DIR="$WEB_DIR/dist"
TASK_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
mkdir -p "$TASK_TARGET_DIR"
TASK_TARGET_DIR="$(cd "$TASK_TARGET_DIR" && pwd)"
export CARGO_TARGET_DIR="$TASK_TARGET_DIR"

# ── Parse arguments ─────────────────────────────────────────
BUILD_MODE="release"
CARGO_FEATURES=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)
            BUILD_MODE="debug"
            shift
            ;;
        --release)
            BUILD_MODE="release"
            shift
            ;;
        --features)
            CARGO_FEATURES="${CARGO_FEATURES:+$CARGO_FEATURES,}$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--debug|--release] [--features <features>]"
            exit 1
            ;;
    esac
done

# ── 1. Clean caches ──────────────────────────────────────────
echo "=== Cleaning caches ==="
rm -rf "$DIST_DIR"
rm -rf "$WEB_DIR/node_modules/.vite"
echo "Done."

# ── 2. Build frontend ────────────────────────────────────────
echo "=== Building frontend ==="
cd "$WEB_DIR"
if [ ! -d "node_modules" ]; then
    npm install
fi
npm run build
echo "Frontend build completed in $DIST_DIR"
ls -R "$DIST_DIR"
cd "$ROOT"

# Build and validate the paired Miao unless an artifact was explicitly supplied.
if [[ -z "${IVNC_MIAO_BIN:-}" ]]; then
    bash "$ROOT/scripts/build-miao.sh" "--$BUILD_MODE"
    export IVNC_MIAO_BIN="$TASK_TARGET_DIR/$BUILD_MODE/miao-rust"
fi

# ── 3. Build backend ─────────────────────────────────────────
CARGO_ARGS=()
if [[ "$BUILD_MODE" == "release" ]]; then
    CARGO_ARGS+=(--release)
fi
if [[ -n "$CARGO_FEATURES" ]]; then
    CARGO_ARGS+=(--features "$CARGO_FEATURES")
fi

echo "=== Building backend ($BUILD_MODE) ==="
cd "$ROOT"
cargo build "${CARGO_ARGS[@]}"
echo "=== Done ==="
cp "$TASK_TARGET_DIR/$BUILD_MODE/ivnc" "$ROOT/ivnc.new"
mv -f "$ROOT/ivnc.new" "$ROOT/ivnc"
echo "Binary: $ROOT/ivnc"
