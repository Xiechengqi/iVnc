#!/usr/bin/env bash
#
# remote_app_smoke.sh — end-to-end regression suite for application-scoped
# remote desktop sharing. Boots two temporary iVnc instances (A and B), turns
# on remote access for a real GTK desktop app on B (which then runs in its own
# runtime), connects from A as a "remote app", and drives a real headless
# Chrome through A's relay. Verifies:
#
#   1. Sharing requires an access password, which B can show again, and
#      authenticates per app.
#   2. Wrong access passwords are rejected; tokens are scoped to one app.
#   3. A remote app can be added on A only with a valid URL + password, and
#      connects when it is started.
#   4. A browser connected only to A receives real video and can upload and
#      send input that reaches the application running on B.
#   5. Further connections (a second A page as an embedded stream tab, and B
#      itself) view the same process in parallel, and every one may control.
#   6. Disabling sharing on B force-disconnects the live remote view; stopping
#      and deleting the remote app on A leave B's application running.
#
# Env knobs (all optional):
#   IVNC_BIN=path     binary to test   (default: target/release/ivnc)
#   KEEP_TMP=1        keep the temporary dir on exit (default: remove)
#   REMOTE_SMOKE_TLS=1 test B's HTTPS/WSS certificate validation as well
#
# Requires: gtk-3-examples, google-chrome, node (with web/ivnc's `ws`), jq,
# curl, python3.

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

# Waits up to $1 seconds for the remaining command to succeed.
eventually() {
  local deadline=$((SECONDS + $1)); shift
  until "$@" >/dev/null 2>&1; do
    [ "$SECONDS" -ge "$deadline" ] && return 1
    sleep 0.5
  done
}

# Prints a field of A's remote app.
remote_field() {
  curl -sS -m 15 "$BASE_A/api/apps/$REMOTE_ID" | jq -r ".$1"
}
remote_state_is() { [ "$(remote_field remote_state)" = "$1" ]; }

free_port() {
  python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()'
}

for tool in gtk3-demo google-chrome node jq curl python3; do
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
    say "KEEP_TMP=1 — temporary dir left at $TMP"
  else
    rm -rf "$TMP"
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

say "binary    : $IVNC_BIN"
say "temp dir  : $TMP"
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
check_eq "sharing is off by default" "false" "$(curl -sS -m 15 -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.sharing.enabled')"

code="$(curl -sS -m 15 -o /dev/null -w '%{http_code}' -u "$B_AUTH" -X PUT "$BASE_B/api/apps/$APP_ID/sharing" \
  -H 'Content-Type: application/json' -d '{"enabled":true}')"
check_eq "enabling without a password is refused" "400" "$code"

# Enabled before the first start, so the app starts in its own runtime.
resp="$(curl -sS -m 15 -u "$B_AUTH" -X PUT "$BASE_B/api/apps/$APP_ID/sharing" \
  -H 'Content-Type: application/json' \
  -d "{\"enabled\":true,\"password\":\"$APP_PASSWORD\"}")"
check_eq "sharing enabled" "true" "$(jq -r '.ok' <<<"$resp")"
check_eq "stopped app needs no restart" "false" "$(jq -r '.restart_required' <<<"$resp")"

resp="$(curl -sS -m 15 -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID/sharing")"
check_eq "sharing reported enabled" "true" "$(jq -r '.enabled' <<<"$resp")"
check_eq "password is viewable on B" "$APP_PASSWORD" "$(jq -r 'select(.password_viewable == true) | .password' <<<"$resp")"
check_eq "share path shape" "/remote/v1/apps/$APP_ID" "$(jq -r '.share_path' <<<"$resp")"
SHARE_URL="$BASE_B$(jq -r '.share_path' <<<"$resp")"

resp="$(curl -sS -m 60 -u "$B_AUTH" -X POST "$BASE_B/api/apps/$APP_ID/start")"
check_eq "app start accepted" "true" "$(jq -r '.ok' <<<"$resp")"
check_eq "shared app runs in its own runtime" "running" "$(curl -sS -m 15 -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.status')"
check_eq "running shared app needs no restart" "false" "$(curl -sS -m 15 -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.sharing.restart_required')"
[ -f "$TMP/b/home/runtimes/$APP_ID/endpoint.json" ] && pass "runtime endpoint published" || fail "runtime endpoint published"

hdr "negative authorization checks"
check_status "remote info without token rejected" "401" "$SHARE_URL/info"
code="$(curl -sS -m 15 -o /dev/null -w '%{http_code}' -X POST "$SHARE_URL/authenticate" \
  -H 'Content-Type: application/json' -d '{"password":"wrong-password"}')"
check_eq "wrong access password rejected" "401" "$code"
resp="$(curl -sS -m 20 -X POST "$BASE_A/api/apps" -H 'Content-Type: application/json' \
  -d "{\"app_type\":\"remote\",\"url\":\"$SHARE_URL\",\"password\":\"wrong-password\"}")"
check_eq "adding remote app with wrong password fails" "null" "$(jq -r '.ok' <<<"$resp")"
token="$(curl -sS -m 15 -X POST "$SHARE_URL/authenticate" -H 'Content-Type: application/json' \
  -d "{\"password\":\"$APP_PASSWORD\"}" | jq -r '.token // empty')"
[ -n "$token" ] && pass "session token issued" || fail "session token issued"
check_status "token reads the shared app" "200" "$SHARE_URL/info" -H "Authorization: Bearer $token"
check_status "token cannot list B applications" "401" "$BASE_B/api/apps" -H "Authorization: Bearer $token"
check_status "view token cannot rename B application" "401" "$BASE_B/api/apps/$APP_ID/name" \
  -X PUT -H "Authorization: Bearer $token" -H 'Content-Type: application/json' -d '{"name":"denied","base_revision":0}'
check_status "cross-origin rename is rejected" "403" "$BASE_B/api/apps/$APP_ID/name" \
  -u "$B_AUTH" -X PUT -H 'Origin: https://invalid.example' -H 'Content-Type: application/json' -d '{"name":"denied","base_revision":0}'
check_status "token cannot access another desktop" "401" "$BASE_B/remote/v1/apps/builtin-chrome/info" -H "Authorization: Bearer $token"
check_status "data proxy rejects management routes" "404" "$SHARE_URL/data/api/apps" -H "Authorization: Bearer $token"
check_status "data proxy offers no file downloads" "404" "$SHARE_URL/data/app/files" -H "Authorization: Bearer $token"

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
check_eq "new remote app is not started" "false" "$(remote_field started)"
check_eq "new remote app is disconnected" "disconnected" "$(remote_field remote_state)"
expected_plaintext=true
if [ "${REMOTE_SMOKE_TLS:-}" = 1 ]; then expected_plaintext=false; fi
check_eq "plaintext connection is marked" "$expected_plaintext" "$(remote_field remote_plaintext)"
check_eq "A lists no password" "null" "$(curl -sS -m 15 "$BASE_A/api/apps" | jq -r --arg id "$REMOTE_ID" '.apps[]|select(.id==$id)|.password')"

resp="$(curl -sS -m 30 -X POST "$BASE_A/api/apps/$REMOTE_ID/start")"
check_eq "remote app start accepted" "true" "$(jq -r '.ok' <<<"$resp")"
check_eq "remote app is started" "true" "$(remote_field started)"
eventually 45 remote_state_is connected && pass "remote app connected" || fail "remote app connected (state: $(remote_field remote_state))"

resp="$(curl -sS -m 20 -X POST "$BASE_A/api/apps/$REMOTE_ID/view")"
VIEW_PATH="$(jq -r '.url' <<<"$resp")"
case "$VIEW_PATH" in
  /app-view/*/) pass "view created" ;;
  *) fail "view creation: $resp"; exit 1 ;;
esac

hdr "browser end-to-end through A"
mkdir -p "$TMP/view"
REMOTE_VIEW_EXPECT_REVOKE=1 node "$SCRIPT_DIR/remote_view_smoke.mjs" \
  "$BASE_A$VIEW_PATH" "$TMP/view" > "$TMP/view-smoke.log" 2>&1 &
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
UPLOADED="$TMP/b/home/apps/$APP_ID/home/Downloads/smoke-upload.txt"
eventually 15 test -s "$UPLOADED"
check_eq "upload reached the app's Downloads on B" "iVnc upload 中文" "$(cat "$UPLOADED" 2>/dev/null)"

hdr "parallel connections"
remote_pid_before="$(curl -sS -m 15 -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.pid')"
remote_order_before="$(curl -sS -m 15 "$BASE_A/api/apps/order")"
resp="$(curl -sS -m 15 -X PUT "$BASE_A/api/apps/$REMOTE_ID/name" \
  -H 'Content-Type: application/json' -d '{"name":"A local alias 中文","base_revision":0}')"
check_eq "remote mapping renamed on A" "A local alias 中文" "$(jq -r '.name' <<<"$resp")"
check_eq "rename increments only the name revision" "1" "$(remote_field name_revision)"
check_eq "A alias rename preserves B application name" "remote-demo" "$(curl -sS -m 15 -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.name')"
check_eq "A alias rename preserves B process" "$remote_pid_before" "$(curl -sS -m 15 -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.pid')"
check_eq "A alias rename preserves display order" "$remote_order_before" "$(curl -sS -m 15 "$BASE_A/api/apps/order")"
check_status "alias rename keeps the existing view token valid" "200" "$BASE_A${VIEW_PATH}ui-config"
check_status "stale legacy alias edit conflicts" "409" "$BASE_A/api/apps/$REMOTE_ID" \
  -X PUT -H 'Content-Type: application/json' -d '{"name":"stale alias","name_revision":0}'
curl -sS -m 15 -X PUT "$BASE_A/api/apps/$REMOTE_ID" \
  -H 'Content-Type: application/json' -d '{"autostart":false}' >/dev/null
check_eq "configuration edit preserves alias" "A local alias 中文" "$(remote_field name)"
check_eq "A reports the active connection" "connected" "$(remote_field remote_state)"
clicks() { local n; n="$(grep -c 'ButtonMask changed' "$APP_LOG" 2>/dev/null)"; printf '%s' "${n:-0}"; }
clicks_increased() { [ "$(clicks)" -gt "$clicks_before" ]; }
clicks_before="$(clicks)"
resp="$(curl -sS -m 20 -X POST "$BASE_A/api/apps/$REMOTE_ID/view")"
SECOND_PATH="$(jq -r '.url' <<<"$resp")"
[ "$SECOND_PATH" != "$VIEW_PATH" ] && pass "each page gets its own view token" || fail "view token reused"
mkdir -p "$TMP/second"
if REMOTE_VIEW_UPLOAD_NAME=second-upload.txt node "$SCRIPT_DIR/remote_view_smoke.mjs" \
  "$BASE_A$SECOND_PATH" "$TMP/second" --embed > "$TMP/second.log" 2>&1; then
  pass "embedded second view receives video, hides its own toolbar and reports its state"
else
  fail "embedded second view"
  cat "$TMP/second.log"
fi
curl -sS -m 5 -X POST "$BASE_A${SECOND_PATH}close" >/dev/null
eventually 10 clicks_increased && pass "second connection may control as well" || fail "second connection input did not reach B"
SECOND_UPLOAD="$TMP/b/home/apps/$APP_ID/home/Downloads/second-upload.txt"
eventually 15 test -s "$SECOND_UPLOAD" && pass "second connection may upload as well" || fail "second connection upload did not reach B"
resp="$(curl -sS -m 20 -u "$B_AUTH" -X POST "$BASE_B/api/apps/$APP_ID/view")"
LOCAL_PATH="$(jq -r '.url' <<<"$resp")"
mkdir -p "$TMP/local"
LOCAL_BASE="${BASE_B/\/\//\/\/$B_AUTH@}"
if node "$SCRIPT_DIR/remote_view_smoke.mjs" "$LOCAL_BASE$LOCAL_PATH" "$TMP/local" --no-input > "$TMP/local.log" 2>&1; then
  pass "B views its shared app while A is connected"
else
  fail "local view on B"
  cat "$TMP/local.log"
fi
curl -sS -m 5 -u "$B_AUTH" -X POST "$BASE_B${LOCAL_PATH}close" >/dev/null
check_eq "connections reuse the same application process" "running" "$(curl -sS -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.status')"

hdr "revocation"
resp="$(curl -sS -m 15 -u "$B_AUTH" -X PUT "$BASE_B/api/apps/$APP_ID/sharing" \
  -H 'Content-Type: application/json' -d '{"enabled":false}')"
check_eq "sharing disabled on B" "true" "$(jq -r '.ok' <<<"$resp")"
check_eq "disabling reports disconnected sessions" "true" "$(jq -r '.revoked' <<<"$resp")"
check_eq "disabling a running shared app needs a restart" "true" "$(jq -r '.restart_required' <<<"$resp")"

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
check_status "old token rejected after disabling" "401" "$SHARE_URL/info" -H "Authorization: Bearer $token"

resp="$(curl -sS -m 20 -X POST "$BASE_A/api/apps/$REMOTE_ID/view")"
check_eq "view rejected after revocation" "null" "$(jq -r '.url' <<<"$resp")"

check_eq "stopping the remote app on A" "true" "$(curl -sS -m 20 -X POST "$BASE_A/api/apps/$REMOTE_ID/stop" | jq -r '.ok')"
check_eq "stopped remote app is not started" "false" "$(remote_field started)"
check_eq "stopped remote app is disconnected" "disconnected" "$(remote_field remote_state)"
check_status "delete remote mapping" "200" "$BASE_A/api/apps/$REMOTE_ID" -X DELETE
check_eq "stopping and deleting on A keep B's application running" "running" "$(curl -sS -u "$B_AUTH" "$BASE_B/api/apps/$APP_ID" | jq -r '.status')"

hdr "summary"
say "passed: $PASS, failed: $FAIL"
if [ "$FAIL" -gt 0 ]; then
  printf 'failed checks:\n'
  for name in "${FAILED_NAMES[@]}"; do printf '  - %s\n' "$name"; done
  exit 1
fi
