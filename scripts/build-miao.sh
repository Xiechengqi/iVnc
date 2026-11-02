#!/usr/bin/env bash
# Build the paired panel before iVnc's build.rs embeds it. Never nest cargo in build.rs.
set -euo pipefail
IVNC_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MIAO_SOURCE_DIR="${MIAO_SOURCE_DIR:-$IVNC_ROOT/../miao}"
BUILD_MODE="${1:---release}"
if [[ "$BUILD_MODE" != --release && "$BUILD_MODE" != --debug ]]; then
    echo "Usage: $0 [--release|--debug]" >&2
    exit 1
fi
if [[ ! -f "$MIAO_SOURCE_DIR/Cargo.toml" ]]; then
    echo "Paired Miao source is missing at $MIAO_SOURCE_DIR. Set MIAO_SOURCE_DIR or IVNC_MIAO_BIN." >&2
    exit 1
fi
TASK_TARGET_DIR="${CARGO_TARGET_DIR:-$IVNC_ROOT/target}"
mkdir -p "$TASK_TARGET_DIR"
TASK_TARGET_DIR="$(cd "$TASK_TARGET_DIR" && pwd)"
export CARGO_TARGET_DIR="$TASK_TARGET_DIR"
cd "$MIAO_SOURCE_DIR/frontend"
if [[ ! -d node_modules ]]; then npm ci; fi
npm run build
cd "$MIAO_SOURCE_DIR"
CARGO_ARGS=()
if [[ "$BUILD_MODE" == --release ]]; then CARGO_ARGS+=(--release); fi
cargo build --locked --bin miao-rust "${CARGO_ARGS[@]}"
PROFILE="${BUILD_MODE#--}"
if [[ "$("$TASK_TARGET_DIR/$PROFILE/miao-rust" --ivnc-integration-version)" != 2 ]]; then
    echo "The paired Miao must support iVnc integration v2 (--nopasswd)" >&2
    exit 1
fi
echo "Paired Miao: $TASK_TARGET_DIR/$PROFILE/miao-rust"
