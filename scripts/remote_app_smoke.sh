#!/usr/bin/env bash
#
# remote_app_smoke.sh — end-to-end regression suite for application-scoped
# remote desktop sharing. Boots two fully isolated iVnc instances (A and B),
# runs a real GTK desktop app inside B's per-application sandbox, shares it
# with an application password, connects from A as a "remote app", and drives
# a real headless Chrome through A's relay. Verifies:
#
#   1. Sharing requires an application password and authenticates per app.
#   2. Wrong application passwords are rejected.
#   3. A remote app can be added on A only with a valid URL + password.
#   4. A browser connected only to A receives real video and can send input
#      that reaches the application running on B.
#   5. Disabling sharing on B force-disconnects the live remote view.
#
# Env knobs (all optional):
#   IVNC_BIN=path     binary to test   (default: target/release/ivnc)
#   KEEP_TMP=1        keep the sandbox dir on exit (default: remove)
#   REMOTE_SMOKE_TLS=1 test B's HTTPS/WSS certificate validation as well
#
# Requires: bubblewrap, slirp4netns, nftables, iproute2, gtk-3-examples,
# google-chrome, node, jq, curl.

set -u -o pipefail

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

check_eq() {
  local name="$1" expected="$2" actual="$3"
  if [ "$expected" = "$actual" ]; then pass "$name"; else fail "$name (expected [$expected], got [$actual])"; fi
}

check_status() {
  local name="$1" expected="$2" url="$3"; shift 3
  local code
  code="$(curl -sS -m 15 -o /dev/null -w '%{http_code}' "$@" "$url")"
  check_eq "$name" "$expected" "$code"
}

free_port() {
  python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()'
}

for tool in bwrap slirp4netns nft ip gtk3-demo google-chrome node jq curl python3; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    say "ERROR: required tool not found: $tool"
    exit 1
  fi
done
if [ ! -x "$IVNC_BIN" ]; then
  say "ERROR: binary not found/executable at $IVNC_BIN (set IVNC_BIN)"
  exit 1
fi

TMP="$(mktemp -d /tmp/ivnc-remote-smoke.XXXXXX)"
PORT_A="$(free_port)"
PORT_B="$(free_port)"
BASE_A="http://127.0.0.1:$PORT_A"
BASE_B="http://127.0.0.1:$PORT_B"
if [ "${REMOTE_SMOKE_TLS:-}" = 1 ]; then
  BASE_B="https://127.0.0.1:$PORT_B"
  export CURL_CA_BUNDLE="$TMP/b/home/tls/cert.pem"
  export IVNC_REMOTE_CA_FILE="$CURL_CA_BUNDLE"
fi
B_AUTH="user:smoke-secret-123"
APP_PASSWORD="app-pass-123"
PID_A=""
PID_B=""

write_config() {
  local dir="$1" port="$2" auth_enabled="$3" auth_password="$4"
  sed -e "s/^port = 8008$/port = $port/" \
      -e 's/^host = "0.0.0.0"$/host = "127.0.0.1"/' \
      -e "s/^basic_auth_enabled = true$/basic_auth_enabled = $auth_enabled/" \
      -e "s/^basic_auth_password = \"mypasswd\"$/basic_auth_password = \"$auth_password\"/" \
      -e 's/^width = 1920$/width = 800/' \
      -e 's/^height = 1080$/height = 600/' \
      config.example.toml > "$dir/config.toml"
}

start_instance() {
  local name="$1" port="$2" auth_enabled="$3" auth_password="$4"
  local dir="$TMP/$name"
  mkdir -p "$dir/run" "$dir/home" "$dir/real-home"
  chmod 700 "$dir/run"
  write_config "$dir" "$port" "$auth_enabled" "$auth_password"
  local tls_args=()
  if [ "$name" = b ] && [ "${REMOTE_SMOKE_TLS:-}" = 1 ]; then tls_args=(--tls); fi
  (
    export XDG_RUNTIME_DIR="$dir/run"
    export IVNC_HOME="$dir/home"
    export HOME="$dir/real-home"
    exec "$IVNC_BIN" -c "$dir/config.toml" --proxy-autostart false "${tls_args[@]}"
  ) > "$dir/server.log" 2>&1 &
  echo $!
}

wait_health() {
  local base="$1" name="$2"
  local i
  for i in $(seq 1 150); do
    if curl -sS -m 2 "$base/health" >/dev/null 2>&1; then return 0; fi
    sleep 0.2
  done
  say "instance $name never became healthy"
  return 1
}

cleanup() {
  local code=$?
  hdr "teardown"
  jobs -p | xargs -r kill 2>/dev/null || true
  for pid in "$PID_A" "$PID_B"; do
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  sleep 1
  for pid in "$PID_A" "$PID_B"; do
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then kill -9 "$pid" 2>/dev/null || true; fi
  done
  if [ "${KEEP_TMP:-}" = "1" ]; then
    say "KEEP_TMP=1 — sandbox left at $TMP"
  else
    rm -rf "$TMP"
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

say "binary    : $IVNC_BIN"
say "sandbox   : $TMP"
say "A         : $BASE_A"
say "B         : $BASE_B"

hdr "boot instances"
PID_B="$(start_instance b "$PORT_B" true "smoke-secret-123")"
PID_A="$(start_instance a "$PORT_A" false "unused")"
wait_health "$BASE_B" B || { tail -n 40 "$TMP/b/server.log"; exit 1; }
wait_health "$BASE_A" A || { tail -n 40 "$TMP/a/server.log"; exit 1; }
pass "both instances healthy"

hdr "desktop app on B + sharing policy"
resp="$(curl -sS -m 15 -u "$B_AUTH" -X POST "$BASE_B/api/apps" \
  -H 'Content-Type: application/json' \
  -d '{"name":"remote-demo","app_type":"desktop","exec_command":"gtk3-demo"}')"
APP_ID="$(jq -r '.app.id' <<<"$resp")"
if [ -n "$APP_ID" ] && [ "$APP_ID" != "null" ]; then pass "desktop app created on B"; else fail "desktop app creation: $resp"; exit 1; fi

resp="$(curl -sS -m 30 -u "$B_AUTH" -X POST "$BASE_B/api/apps/$APP_ID/start")"
check_eq "app start accepted" "true" "$(jq -r '.ok' <<<"$resp")"

resp="$(curl -sS -m 15 -u "$B_AUTH" -X PUT "$BASE_B/api/apps/$APP_ID/sharing" \
  -H 'Content-Type: application/json' \
  -d "{\"enabled\":true,\"password\":\"$APP_PASSWORD\",\"desktop\":{\"x11\":false,\"services\":[]}}")"
check_eq "sharing enabled" "true" "$(jq -r '.policy.enabled' <<<"$resp")"
check_eq "password stored as hash only" "true" "$(jq -r '.policy.password_set' <<<"$resp")"

resp="$(curl -sS -m 15 -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID/sharing")"
SHARE_URL="$BASE_B$(jq -r '.share_path' <<<"$resp")"
check_eq "share path shape" "/remote/v1/apps/$APP_ID" "$(jq -r '.share_path' <<<"$resp")"

hdr "negative authorization checks"
check_status "remote info without credential rejected" "401" "$SHARE_URL/info"
code="$(curl -sS -m 15 -o /dev/null -w '%{http_code}' -X POST "$SHARE_URL/authenticate" \
  -H 'Content-Type: application/json' -d '{"password":"wrong-password"}')"
check_eq "wrong application password rejected" "401" "$code"
resp="$(curl -sS -m 20 -X POST "$BASE_A/api/apps" -H 'Content-Type: application/json' \
  -d "{\"app_type\":\"remote\",\"url\":\"$SHARE_URL\",\"password\":\"wrong-password\"}")"
check_eq "adding remote app with wrong password fails" "null" "$(jq -r '.ok' <<<"$resp")"
credential="$(curl -sS -m 15 -X POST "$SHARE_URL/authenticate" -H 'Content-Type: application/json' \
  -d "{\"password\":\"$APP_PASSWORD\"}" | jq -r '.credential')"
check_status "app credential cannot list B applications" "401" "$BASE_B/api/apps" -H "Authorization: Bearer $credential"
check_status "app credential cannot access another desktop" "401" "$BASE_B/remote/v1/apps/builtin-chrome/info" -H "Authorization: Bearer $credential"
check_status "data proxy rejects management routes" "404" "$SHARE_URL/data/api/apps" -H "Authorization: Bearer $credential"
check_status "temporary credential revocation" "200" "$SHARE_URL/credential" -X DELETE -H "Authorization: Bearer $credential"
check_status "revoked credential is rejected" "401" "$SHARE_URL/info" -H "Authorization: Bearer $credential"

hdr "remote app on A"
resp="$(curl -sS -m 20 -X POST "$BASE_A/api/remote/test" -H 'Content-Type: application/json' \
  -d "{\"url\":\"$SHARE_URL\",\"password\":\"$APP_PASSWORD\"}")"
check_eq "test connection reports running app" "running" "$(jq -r '.status' <<<"$resp")"

resp="$(curl -sS -m 20 -X POST "$BASE_A/api/apps" -H 'Content-Type: application/json' \
  -d "{\"app_type\":\"remote\",\"url\":\"$SHARE_URL\",\"password\":\"$APP_PASSWORD\"}")"
REMOTE_ID="$(jq -r '.app.id' <<<"$resp")"
REMOTE_NAME="$(jq -r '.app.name' <<<"$resp")"
if [ -n "$REMOTE_ID" ] && [ "$REMOTE_ID" != "null" ]; then pass "remote app added on A (name: $REMOTE_NAME)"; else fail "remote app creation: $resp"; exit 1; fi
check_eq "remote app defaults to target name" "remote-demo" "$REMOTE_NAME"

resp="$(curl -sS -m 20 -X POST "$BASE_A/api/apps/$REMOTE_ID/view")"
VIEW_PATH="$(jq -r '.url' <<<"$resp")"
case "$VIEW_PATH" in
  /app-view/*/) pass "view created" ;;
  *) fail "view creation: $resp"; exit 1 ;;
esac

hdr "browser end-to-end through A"
mkdir -p "$TMP/view"
REMOTE_VIEW_EXPECT_REVOKE=1 node "$SCRIPT_DIR/remote_view_smoke.mjs" \
  "$BASE_A$VIEW_PATH?app=$REMOTE_ID" "$TMP/view" > "$TMP/view-smoke.log" 2>&1 &
VIEW_PID=$!

input_ok=""
for _ in $(seq 1 240); do
  if grep -q 'PASS input-click-sent' "$TMP/view-smoke.log" 2>/dev/null; then input_ok=1; break; fi
  if ! kill -0 "$VIEW_PID" 2>/dev/null; then break; fi
  sleep 0.5
done
if [ "$input_ok" = "1" ]; then
  pass "browser connected and received video through relay"
else
  fail "browser did not reach input stage"
  cat "$TMP/view-smoke.log"
fi

APP_LOG="$TMP/b/home/apps/$APP_ID/app.log"
click_ok=""
for _ in $(seq 1 20); do
  if grep -q 'ButtonMask changed' "$APP_LOG" 2>/dev/null; then click_ok=1; break; fi
  sleep 0.5
done
check_eq "mouse click reached application compositor on B" "1" "$click_ok"

hdr "concurrent viewers and owner control"
check_eq "A reports the active connection" "connected" "$(curl -sS "$BASE_A/api/apps/$REMOTE_ID" | jq -r '.status')"
resp="$(curl -sS -m 20 -X POST "$BASE_A/api/apps/$REMOTE_ID/view")"
VIEWER_PATH="$(jq -r '.url' <<<"$resp")"
mkdir -p "$TMP/viewer"
if node "$SCRIPT_DIR/remote_view_smoke.mjs" "$BASE_A$VIEWER_PATH?app=$REMOTE_ID" "$TMP/viewer" --viewer > "$TMP/viewer.log" 2>&1; then
  pass "second viewer receives video; server rejects takeover, resize and upload"
else
  fail "second viewer control boundary"
  cat "$TMP/viewer.log"
fi
curl -sS -m 5 -X POST "$BASE_A${VIEWER_PATH}close" >/dev/null
resp="$(curl -sS -m 20 -u "$B_AUTH" -X POST "$BASE_B/api/apps/$APP_ID/view")"
OWNER_PATH="$(jq -r '.url' <<<"$resp")"
mkdir -p "$TMP/owner"
OWNER_BASE="${BASE_B/\/\//\/\/$B_AUTH@}"
if node "$SCRIPT_DIR/remote_view_smoke.mjs" "$OWNER_BASE$OWNER_PATH?app=$APP_ID" "$TMP/owner" --owner > "$TMP/owner.log" 2>&1; then
  pass "B owner can take control from remote controller"
else
  fail "owner takeover"
  cat "$TMP/owner.log"
fi
curl -sS -m 5 -u "$B_AUTH" -X POST "$BASE_B${OWNER_PATH}close" >/dev/null
check_eq "viewers reuse the same application process" "running" "$(curl -sS -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.status')"

hdr "revocation"
resp="$(curl -sS -m 15 -u "$B_AUTH" -X PUT "$BASE_B/api/apps/$APP_ID/sharing" \
  -H 'Content-Type: application/json' \
  -d '{"enabled":false,"desktop":{"x11":false,"services":[]}}')"
check_eq "sharing disabled on B" "false" "$(jq -r '.policy.enabled' <<<"$resp")"

revoke_ok=""
for _ in $(seq 1 60); do
  if ! kill -0 "$VIEW_PID" 2>/dev/null; then revoke_ok=1; break; fi
  sleep 0.5
done
if [ "$revoke_ok" = "1" ] && grep -q 'PASS revocation-disconnect' "$TMP/view-smoke.log" 2>/dev/null; then
  pass "live view force-disconnected after sharing disabled"
else
  fail "view was not disconnected after revocation"
  cat "$TMP/view-smoke.log"
fi

resp="$(curl -sS -m 20 -X POST "$BASE_A/api/apps/$REMOTE_ID/view")"
check_eq "view rejected after revocation" "null" "$(jq -r '.url' <<<"$resp")"

check_status "delete remote mapping" "200" "$BASE_A/api/apps/$REMOTE_ID" -X DELETE
check_eq "deleting mapping keeps B's application running" "running" "$(curl -sS -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.status')"

hdr "summary"
say "passed: $PASS, failed: $FAIL"
if [ "$FAIL" -gt 0 ]; then
  printf 'failed checks:\n'
  for name in "${FAILED_NAMES[@]}"; do printf '  - %s\n' "$name"; done
  exit 1
fi
