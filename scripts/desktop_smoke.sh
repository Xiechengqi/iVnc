#!/usr/bin/env bash
#
# desktop_smoke.sh — deterministic, secret-free regression smoke suite for the
# iVnc desktop, app management and capability surface. Run after every major
# change.
#
# It boots a fully isolated ivnc instance (own XDG_RUNTIME_DIR / config / data
# dirs, own HTTP port, own Chrome DevTools port, basic-auth disabled) and
# exercises the HTTP surface. It needs no API keys and contacts no external
# network. All output goes to stdout.
#
# What it covers:
#   1. Console API: overview, settings round-trip (persisted to console.json),
#      keyframe request.
#   2. Apps CRUD: create / list / fetch / update / delete a CLI app.
#   3. Built-in apps: builtin-chrome and builtin-agent-browser are seeded.
#   4. Capabilities: snapshot, apps, tools, calls; the raw CLI tool for a
#      registered CLI app shows up in the tool list.
#   5. Removed surface: agent, MCP, provider and schedule endpoints are gone.
#   6. WebRTC signaling: the /webrtc websocket accepts a connection.
#
# Env knobs (all optional):
#   IVNC_BIN=path      binary to test          (default: target/release/ivnc)
#   SKIP_BUILD=1       skip `cargo build`      (default: build if not SKIP_BUILD)
#   SKIP_UNIT=1        skip `cargo test`       (default: run if not SKIP_UNIT)
#   SKIP_CHROME=1      skip the Chrome launch  (default: run it)
#   KEEP_TMP=1         keep the temp sandbox dir on exit (default: remove)
#
# Exit code: 0 if all checks pass, 1 otherwise.

set -u

# ----------------------------------------------------------------------------
# Locate repo root (this script lives in <root>/scripts/).
# ----------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

IVNC_BIN="${IVNC_BIN:-$ROOT/target/release/ivnc}"

PASS=0
FAIL=0
FAILED_NAMES=()

say()  { printf '%s\n' "$*"; }
hdr()  { printf '\n=== %s ===\n' "$*"; }
pass() { PASS=$((PASS+1)); printf '  [PASS] %s\n' "$*"; }
fail() { FAIL=$((FAIL+1)); FAILED_NAMES+=("$*"); printf '  [FAIL] %s\n' "$*"; }

# check_eq NAME EXPECTED ACTUAL — equality assertion
check_eq() {
  local name="$1" expected="$2" actual="$3"
  if [ "$expected" = "$actual" ]; then
    pass "$name"
  else
    fail "$name (expected [$expected], got [$actual])"
  fi
}

# check_contains NAME NEEDLE HAYSTACK
check_contains() {
  local name="$1" needle="$2" haystack="$3"
  case "$haystack" in
    *"$needle"*) pass "$name" ;;
    *) fail "$name (missing [$needle] in: $(printf '%.200s' "$haystack"))" ;;
  esac
}

# check_status NAME EXPECTED_CODE URL [curl args...]
check_status() {
  local name="$1" expected="$2" url="$3"; shift 3
  local code
  code="$(curl -sS -m 10 -o /dev/null -w '%{http_code}' "$@" "$url")"
  check_eq "$name" "$expected" "$code"
}

free_port() {
  python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()'
}

# ----------------------------------------------------------------------------
# Optional build + unit tests.
# ----------------------------------------------------------------------------
if [ "${SKIP_BUILD:-}" != "1" ]; then
  hdr "cargo build --release"
  if cargo build --release; then
    pass "cargo build"
  else
    fail "cargo build"
    say "build failed — aborting"; exit 1
  fi
else
  say "SKIP_BUILD=1 — using existing binary"
fi

if [ ! -x "$IVNC_BIN" ]; then
  say "ERROR: binary not found/executable at $IVNC_BIN (set IVNC_BIN or unset SKIP_BUILD)"
  exit 1
fi

if [ "${SKIP_UNIT:-}" != "1" ]; then
  hdr "cargo test"
  if cargo test; then
    pass "cargo test"
  else
    fail "cargo test"
  fi
else
  say "SKIP_UNIT=1 — skipping cargo test"
fi

# ----------------------------------------------------------------------------
# Isolated sandbox: own runtime/config/data dirs + ports.
# ----------------------------------------------------------------------------
TMP="$(mktemp -d /tmp/ivnc-smoke.XXXXXX)"
export XDG_RUNTIME_DIR="$TMP/run"
export XDG_CONFIG_HOME="$TMP/config"
export XDG_DATA_HOME="$TMP/data"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
chmod 700 "$XDG_RUNTIME_DIR"  # Wayland refuses a runtime dir that isn't 0700

HTTP_PORT="$(free_port)"
DEBUG_PORT="$(free_port)"
export IVNC_CHROME_DEBUG_PORT="$DEBUG_PORT"
BASE="http://127.0.0.1:$HTTP_PORT"
IVNC_PID=""

cleanup() {
  local code=$?
  hdr "teardown"
  curl -sS -m 5 -X POST "$BASE/api/apps/builtin-chrome/stop" >/dev/null 2>&1 || true
  pkill -f "remote-debugging-port=$DEBUG_PORT" >/dev/null 2>&1 || true
  if [ -n "$IVNC_PID" ] && kill -0 "$IVNC_PID" 2>/dev/null; then
    kill "$IVNC_PID" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      kill -0 "$IVNC_PID" 2>/dev/null || break
      sleep 0.3
    done
    kill -9 "$IVNC_PID" 2>/dev/null || true
  fi
  if [ "${KEEP_TMP:-}" = "1" ]; then
    say "KEEP_TMP=1 — sandbox left at $TMP (ivnc log: $TMP/ivnc.log)"
  else
    rm -rf "$TMP"
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

hdr "boot isolated ivnc"
say "binary      : $IVNC_BIN"
say "http port   : $HTTP_PORT"
say "debug port  : $DEBUG_PORT"
say "sandbox     : $TMP"
"$IVNC_BIN" --http-port "$HTTP_PORT" --basic-auth-enabled false \
  > "$TMP/ivnc.log" 2>&1 &
IVNC_PID=$!
say "ivnc pid    : $IVNC_PID"

READY=0
for _ in $(seq 1 60); do
  if ! kill -0 "$IVNC_PID" 2>/dev/null; then
    say "ERROR: ivnc exited during startup — last log lines:"
    tail -n 30 "$TMP/ivnc.log" || true
    fail "ivnc boot"
    exit 1
  fi
  if curl -sS -m 2 "$BASE/api/console/overview" >/dev/null 2>&1; then
    READY=1; break
  fi
  sleep 0.5
done
if [ "$READY" = "1" ]; then pass "ivnc boot (HTTP ready)"; else
  fail "ivnc boot (HTTP never became ready)"; tail -n 30 "$TMP/ivnc.log" || true; exit 1
fi

# ----------------------------------------------------------------------------
# 1. Console API.
# ----------------------------------------------------------------------------
hdr "console: overview"
resp="$(curl -sS -m 10 "$BASE/api/console/overview")"
check_contains "overview has version"  '"version"'  "$resp"
check_contains "overview has display"  '"display"'  "$resp"
check_contains "overview has runtime"  '"runtime"'  "$resp"
check_eq "overview drops agent block"     "null" "$(jq -r '.agent // "null"'     <<<"$resp")"
check_eq "overview drops providers block" "null" "$(jq -r '.providers // "null"' <<<"$resp")"

hdr "console: settings round-trip"
resp="$(curl -sS -m 10 "$BASE/api/console/settings")"
check_contains "settings has saved+current" '"current"' "$resp"

curl -sS -m 10 -X PUT "$BASE/api/console/settings" \
  -H 'content-type: application/json' \
  -d '{"target_fps":24,"video_bitrate_kbps":4321}' >/dev/null
resp="$(curl -sS -m 10 "$BASE/api/console/settings")"
check_eq "settings applied (fps)"     "24"   "$(jq -r '.current.target_fps'        <<<"$resp")"
check_eq "settings applied (bitrate)" "4321" "$(jq -r '.current.video_bitrate_kbps' <<<"$resp")"
check_eq "settings persisted (fps)"   "24"   "$(jq -r '.saved.target_fps'          <<<"$resp")"
check_eq "console.json written" "24" \
  "$(jq -r '.runtime.target_fps' "$XDG_CONFIG_HOME/ivnc/console.json" 2>/dev/null)"

check_status "keyframe request 200" "200" "$BASE/api/console/keyframe" -X POST

# ----------------------------------------------------------------------------
# 2. Apps CRUD.
# ----------------------------------------------------------------------------
hdr "apps: built-ins seeded"
resp="$(curl -sS -m 10 "$BASE/api/apps")"
check_contains "builtin-chrome present"        '"builtin-chrome"'        "$resp"
check_contains "builtin-agent-browser present" '"builtin-agent-browser"' "$resp"
check_eq "apps carry no skill_paths" "0" "$(grep -c 'skill_paths' <<<"$resp")"

hdr "apps: CRUD on a CLI app"
resp="$(curl -sS -m 10 -X POST "$BASE/api/apps" -H 'content-type: application/json' \
  -d '{"name":"smoke-cli","app_type":"cli","cli_binary_path":"/bin/echo","cli_env_vars":{"NO_COLOR":"1"}}')"
APP_ID="$(jq -r '.id // .app.id // empty' <<<"$resp")"
if [ -n "$APP_ID" ]; then pass "create CLI app ($APP_ID)"; else
  fail "create CLI app (resp: $(printf '%.200s' "$resp"))"; APP_ID="smoke-cli"; fi

resp="$(curl -sS -m 10 "$BASE/api/apps/$APP_ID")"
check_contains "fetch created app" '"smoke-cli"' "$resp"
check_eq "cli_binary_path stored" "/bin/echo" "$(jq -r '.cli_binary_path // .app.cli_binary_path' <<<"$resp")"

check_status "update app 200" "200" "$BASE/api/apps/$APP_ID" \
  -X PUT -H 'content-type: application/json' \
  -d '{"name":"smoke-cli","app_type":"cli","cli_binary_path":"/bin/true"}'
resp="$(curl -sS -m 10 "$BASE/api/apps/$APP_ID")"
check_eq "update took effect" "/bin/true" "$(jq -r '.cli_binary_path // .app.cli_binary_path' <<<"$resp")"

# ----------------------------------------------------------------------------
# 3. Capabilities.
# ----------------------------------------------------------------------------
hdr "capabilities: snapshot"
resp="$(curl -sS -m 10 "$BASE/api/capabilities")"
check_contains "snapshot has apps"  '"apps"'  "$resp"
check_contains "snapshot has tools" '"tools"' "$resp"
check_eq "snapshot has no skills" "null" "$(jq -r '.skills // "null"' <<<"$resp")"

resp="$(curl -sS -m 10 "$BASE/api/capabilities/tools")"
check_contains "smoke-cli raw tool listed" "$APP_ID" "$(jq -c '.tools' <<<"$resp")"

check_status "capabilities/apps 200"  "200" "$BASE/api/capabilities/apps"
check_status "capabilities/calls 200" "200" "$BASE/api/capabilities/calls"
check_status "capabilities/skills gone" "404" "$BASE/api/capabilities/skills"

hdr "apps: delete"
check_status "delete app 200" "200" "$BASE/api/apps/$APP_ID" -X DELETE
check_status "deleted app 404" "404" "$BASE/api/apps/$APP_ID"

# ----------------------------------------------------------------------------
# 4. Removed surface must be gone.
# ----------------------------------------------------------------------------
hdr "removed endpoints return 404"
check_status "/mcp gone"                    "404" "$BASE/mcp"
check_status "agent-config gone"            "404" "$BASE/api/console/agent-config"
check_status "agent-runs gone"              "404" "$BASE/api/console/agent-runs"
check_status "providers gone"               "404" "$BASE/api/console/providers"
check_status "schedules gone"               "404" "$BASE/api/console/schedules"
check_status "agent-start gone"             "404" "$BASE/api/console/agent-start" -X POST

# ----------------------------------------------------------------------------
# 5. Core desktop surface still up.
# ----------------------------------------------------------------------------
hdr "core endpoints"
check_status "health 200"     "200" "$BASE/health"
check_status "version 200"    "200" "$BASE/version"
check_status "console page"   "200" "$BASE/console"
check_status "desktop page"   "200" "$BASE/"
check_status "ui-config 200"  "200" "$BASE/ui-config"

hdr "webrtc signaling handshake"
if python3 - "$HTTP_PORT" <<'PY'
import socket, sys, base64, os
port = int(sys.argv[1])
key = base64.b64encode(os.urandom(16)).decode()
req = (f"GET /webrtc HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
       "Upgrade: websocket\r\nConnection: Upgrade\r\n"
       f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n")
s = socket.create_connection(("127.0.0.1", port), timeout=5)
s.sendall(req.encode())
resp = s.recv(256).decode(errors="replace")
s.close()
sys.exit(0 if "101" in resp.split("\r\n")[0] else 1)
PY
then pass "/webrtc websocket upgrade (101)"; else fail "/webrtc websocket upgrade"; fi

# ----------------------------------------------------------------------------
# 6. Chrome launch (optional).
# ----------------------------------------------------------------------------
if [ "${SKIP_CHROME:-}" != "1" ]; then
  hdr "apps: launch built-in Chrome"
  check_status "start chrome 200" "200" "$BASE/api/apps/builtin-chrome/start" -X POST
  UP=0
  for _ in $(seq 1 40); do
    if curl -sS -m 2 "http://127.0.0.1:$DEBUG_PORT/json/version" >/dev/null 2>&1; then UP=1; break; fi
    sleep 0.5
  done
  if [ "$UP" = "1" ]; then pass "chrome devtools reachable on $DEBUG_PORT"; else
    fail "chrome devtools never came up"; fi
  check_status "stop chrome 200" "200" "$BASE/api/apps/builtin-chrome/stop" -X POST
else
  say "SKIP_CHROME=1 — skipping Chrome launch"
fi

# ----------------------------------------------------------------------------
# Summary.
# ----------------------------------------------------------------------------
hdr "summary"
say "passed: $PASS"
say "failed: $FAIL"
if [ "$FAIL" -gt 0 ]; then
  for n in "${FAILED_NAMES[@]}"; do say "  - $n"; done
  exit 1
fi
exit 0
