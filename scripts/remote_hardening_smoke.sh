#!/usr/bin/env bash
# End-to-end checks for desktop application runtimes and remote access
# management. Boots two temporary instances (A and B); never touches running ones.
#
#   1. A launcher that forks and exits keeps its application running.
#   2. A crashing application is restarted with backoff, not in a tight loop.
#   3. Turning remote access on or off for a running app asks for a restart;
#      the restart switches between the main desktop and the app's own runtime,
#      and the app sees the same files, network and environment in both.
#   4. B shows the access password; changing it disconnects remote sessions,
#      A reports that the password is required and accepts the new one.
#   5. A follows B's application state; mapping B onto itself is refused.
#   6. Failed authentication is limited per source and application.
#   7. TLS rejects a certificate without its key.
#
# Usage: IVNC_BIN=target/release/ivnc bash scripts/remote_hardening_smoke.sh
#   KEEP_TMP=1  keep the temporary directory

set -u -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"
IVNC_BIN="${IVNC_BIN:-$ROOT/target/release/ivnc}"

PASS=0
FAIL=0
say()  { printf '%s\n' "$*"; }
hdr()  { printf '\n=== %s ===\n' "$*"; }
pass() { PASS=$((PASS+1)); printf '  [PASS] %s\n' "$*"; }
fail() { FAIL=$((FAIL+1)); printf '  [FAIL] %s\n' "$*"; }
check_eq() {
  if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (expected [$2], got [$3])"; fi
}
free_port() {
  python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()'
}
# Waits up to $1 seconds for the remaining command to succeed.
eventually() {
  local deadline=$((SECONDS + $1)); shift
  until "$@" >/dev/null 2>&1; do
    [ "$SECONDS" -ge "$deadline" ] && return 1
    sleep 0.5
  done
}

for tool in jq curl python3; do
  command -v "$tool" >/dev/null 2>&1 || { say "ERROR: required tool not found: $tool"; exit 1; }
done
[ -x "$IVNC_BIN" ] || { say "ERROR: binary not found at $IVNC_BIN"; exit 1; }

TMP="$(mktemp -d /tmp/ivnc-hardening-smoke.XXXXXX)"
PORT_A="$(free_port)"
PORT_B="$(free_port)"
BASE_A="http://127.0.0.1:$PORT_A"
BASE_B="http://127.0.0.1:$PORT_B"
B_AUTH="user:smoke-secret-123"
APP_PASSWORD="app-pass-123"
PIDS=()

start_instance() {
  local name="$1" port="$2" auth="$3"
  local dir="$TMP/$name"
  mkdir -p "$dir/run" "$dir/home" "$dir/real-home"
  chmod 700 "$dir/run"
  sed -e "s/^port = 8008$/port = $port/" \
      -e 's/^host = "0.0.0.0"$/host = "127.0.0.1"/' \
      -e "s/^basic_auth_enabled = true$/basic_auth_enabled = $auth/" \
      -e 's/^basic_auth_password = "mypasswd"$/basic_auth_password = "smoke-secret-123"/' \
      -e 's/^width = 1920$/width = 800/' -e 's/^height = 1080$/height = 600/' \
      config.example.toml > "$dir/config.toml"
  (
    export XDG_RUNTIME_DIR="$dir/run" IVNC_HOME="$dir/home" HOME="$dir/real-home"
    export TZ="Asia/Shanghai" LANG="C.UTF-8" SMOKE_SECRET="manager-only"
    exec "$IVNC_BIN" -c "$dir/config.toml" --proxy-autostart false
  ) > "$dir/server.log" 2>&1 &
  PIDS+=($!)
}
cleanup() {
  local code=$?
  hdr "teardown"
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  sleep 2
  for pid in "${PIDS[@]}"; do kill -9 "$pid" 2>/dev/null || true; done
  if [ "${KEEP_TMP:-}" = 1 ]; then say "left at $TMP"; else rm -rf "$TMP"; fi
  say "passed: $PASS, failed: $FAIL"
  [ "$FAIL" -eq 0 ] && exit "$code" || exit 1
}
trap cleanup EXIT INT TERM

b() { curl -sS -m 30 -u "$B_AUTH" -H 'Content-Type: application/json' "$@"; }
a() { curl -sS -m 30 -H 'Content-Type: application/json' "$@"; }
add_app() {
  b -X POST "$BASE_B/api/apps" -d "$(jq -nc --arg n "$1" --arg c "$2" '{name:$n,app_type:"desktop",exec_command:$c}')" | jq -r '.app.id'
}
policy() { b -X PUT "$BASE_B/api/apps/$1/sharing" -d "$2"; }
home_of() { printf '%s/b/home/apps/%s/home' "$TMP" "$1"; }
field_b() { b "$BASE_B/api/apps/$1" | jq -r "$2"; }
field_a() { a "$BASE_A/api/apps/$1" | jq -r "$2"; }
remote_state_is() { [ "$(field_a "$1" .remote_state)" = "$2" ]; }
status_is() { [ "$(field_b "$1" .status)" = "$2" ]; }
authenticate() {
  curl -sS -m 15 -X POST "$1/authenticate" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg p "$2" '{password:$p}')" | jq -r '.token // empty'
}
info_code() { curl -sS -m 15 -o /dev/null -w '%{http_code}' "$1/info" -H "Authorization: Bearer $2"; }

hdr "boot"
start_instance b "$PORT_B" true
start_instance a "$PORT_A" false
eventually 30 curl -sf "$BASE_B/health" && eventually 30 curl -sf "$BASE_A/health" \
  && pass "instances healthy" || { fail "instances healthy"; tail -20 "$TMP/b/server.log"; exit 1; }

hdr "launcher that forks and exits"
LAUNCHER="$(add_app launcher '(sleep 600 &) ; exit 0')"
check_eq "launcher app starts" "true" "$(b -X POST "$BASE_B/api/apps/$LAUNCHER/start" | jq -r '.ok')"
sleep 6
check_eq "forked application keeps running" "running" "$(field_b "$LAUNCHER" .status)"
b -X POST "$BASE_B/api/apps/$LAUNCHER/stop" >/dev/null
sleep 1
check_eq "stopped launcher app" "stopped" "$(field_b "$LAUNCHER" .status)"

hdr "crash backoff"
CRASHER="$(add_app crasher 'sleep 4; exit 3')"
check_eq "crashing app starts" "true" "$(b -X POST "$BASE_B/api/apps/$CRASHER/start" | jq -r '.ok')"
sleep 20
starts="$(grep -c "restarted app 'crasher'" "$TMP/b/server.log")"
[ "$starts" -ge 1 ] && [ "$starts" -le 2 ] && pass "crashing app restarted with backoff ($starts restarts in 20s)" || fail "crash loop: $starts restarts in 20s"
grep -q "app $CRASHER exited unexpectedly; restarting in" "$TMP/b/server.log" && pass "backoff delay logged" || fail "backoff delay logged"
check_eq "crashing app reported" "crashed" "$(field_b "$CRASHER" .status)"
b -X POST "$BASE_B/api/apps/$CRASHER/stop" >/dev/null

hdr "run mode follows remote access"
SHARED="$TMP/host-data"
mkdir -p "$SHARED"
echo "visible" > "$SHARED/marker"
# Records what the app sees on every start: host files, host loopback, the
# manager's environment (minus iVnc's own session) and its HOME.
PROBE='cat '"$SHARED"'/marker > "$HOME/seen" 2>&1; echo "$TZ|$LANG|${SMOKE_SECRET:-}|$HOME" > "$HOME/env"; curl --noproxy "*" -s -m 3 -o /dev/null -w "%{http_code}" '"$BASE_B"'/health > "$HOME/net" 2>&1; exec sleep 600'
PROBE_APP="$(add_app probe "$PROBE")"
PROBE_HOME="$(home_of "$PROBE_APP")"
RUNTIME="$TMP/b/home/runtimes/$PROBE_APP"
# Called before every (re)start, so the next check sees only the new run.
reset_probe() { rm -f "$PROBE_HOME/seen" "$PROBE_HOME/net" "$PROBE_HOME/env"; }
probe_sees_host() {
  local mode="$1"
  eventually 20 test -s "$PROBE_HOME/net"
  check_eq "$mode: reads host files" "visible" "$(cat "$PROBE_HOME/seen" 2>/dev/null)"
  check_eq "$mode: reaches host loopback" "200" "$(cat "$PROBE_HOME/net" 2>/dev/null)"
  check_eq "$mode: inherits the manager environment and app HOME" \
    "Asia/Shanghai|C.UTF-8|manager-only|$PROBE_HOME" "$(cat "$PROBE_HOME/env" 2>/dev/null)"
}
reset_probe
check_eq "local app starts" "true" "$(b -X POST "$BASE_B/api/apps/$PROBE_APP/start" | jq -r '.ok')"
check_eq "local app is running" "running" "$(field_b "$PROBE_APP" .status)"
probe_sees_host "main desktop"
[ ! -e "$RUNTIME/endpoint.json" ] && pass "local app has no runtime endpoint" || fail "local app has no runtime endpoint"

code="$(b -o /dev/null -w '%{http_code}' -X PUT "$BASE_B/api/apps/$PROBE_APP/sharing" -d '{"enabled":true}')"
check_eq "enabling without a password is refused" "400" "$code"
code="$(b -o /dev/null -w '%{http_code}' -X PUT "$BASE_B/api/apps/$PROBE_APP/sharing" -d '{"enabled":true,"password":"short"}')"
check_eq "short password is refused" "400" "$code"
resp="$(policy "$PROBE_APP" "{\"enabled\":true,\"password\":\"$APP_PASSWORD\"}")"
check_eq "sharing enabled" "true" "$(jq -r '.ok' <<<"$resp")"
check_eq "running app needs a restart" "true" "$(jq -r '.restart_required' <<<"$resp")"
check_eq "app list shows the pending restart" "true|true" "$(field_b "$PROBE_APP" '"\(.sharing.enabled)|\(.sharing.restart_required)"')"
SHARE="$BASE_B/remote/v1/apps/$PROBE_APP"
token="$(authenticate "$SHARE" "$APP_PASSWORD")"
[ -n "$token" ] && pass "token issued before restart" || fail "token issued before restart"
check_eq "B reports not ready until restart" "false" "$(curl -sS -m 15 "$SHARE/info" -H "Authorization: Bearer $token" | jq -r '.ready')"

reset_probe
check_eq "restart accepted" "true" "$(b -X POST "$BASE_B/api/apps/$PROBE_APP/restart" | jq -r '.ok')"
eventually 30 status_is "$PROBE_APP" running && pass "shared app running in its own runtime" || fail "shared app running (status: $(field_b "$PROBE_APP" .status))"
check_eq "no restart pending" "false" "$(field_b "$PROBE_APP" .sharing.restart_required)"
[ -s "$RUNTIME/endpoint.json" ] && pass "runtime endpoint published" || fail "runtime endpoint published"
[ ! -e "$RUNTIME/launch.json" ] && pass "launch description removed after start" || fail "launch description removed after start"
probe_sees_host "own runtime"
check_eq "B reports ready after restart" "true" "$(curl -sS -m 15 "$SHARE/info" -H "Authorization: Bearer $token" | jq -r '.ready')"

hdr "access password"
resp="$(b "$BASE_B/api/apps/$PROBE_APP/sharing")"
check_eq "password is viewable" "true|$APP_PASSWORD" "$(jq -r '"\(.password_viewable)|\(.password)"' <<<"$resp")"
check_eq "unchanged password keeps sessions" "false" "$(policy "$PROBE_APP" '{"enabled":true}' | jq -r '.revoked')"
check_eq "token still valid" "200" "$(info_code "$SHARE" "$token")"

hdr "remote app on A"
resp="$(a -X POST "$BASE_A/api/apps" -d "{\"app_type\":\"remote\",\"url\":\"$SHARE\",\"password\":\"$APP_PASSWORD\"}")"
REMOTE="$(jq -r '.app.id' <<<"$resp")"
[ -n "$REMOTE" ] && [ "$REMOTE" != null ] && pass "remote app added on A" || fail "remote app added on A: $resp"
check_eq "remote app starts" "true" "$(a -X POST "$BASE_A/api/apps/$REMOTE/start" | jq -r '.ok')"
eventually 45 remote_state_is "$REMOTE" connected && pass "A connects to B" || fail "A connects to B (state: $(field_a "$REMOTE" .remote_state))"

NEW_PASSWORD="app-pass-456"
resp="$(policy "$PROBE_APP" "{\"enabled\":true,\"password\":\"$NEW_PASSWORD\"}")"
check_eq "password change disconnects sessions" "true" "$(jq -r '.revoked' <<<"$resp")"
check_eq "password change needs no restart" "false" "$(jq -r '.restart_required' <<<"$resp")"
check_eq "old token rejected" "401" "$(info_code "$SHARE" "$token")"
check_eq "old password rejected" "" "$(authenticate "$SHARE" "$APP_PASSWORD")"
code="$(a -o /dev/null -w '%{http_code}' -X POST "$BASE_A/api/apps/$REMOTE/view")"
[ "$code" -ge 400 ] && pass "view refused with the old password ($code)" || fail "view refused with the old password ($code)"
eventually 45 remote_state_is "$REMOTE" auth_required && pass "A asks for the password again" || fail "A asks for the password again (state: $(field_a "$REMOTE" .remote_state))"
code="$(a -o /dev/null -w '%{http_code}' -X PUT "$BASE_A/api/apps/$REMOTE" -d "{\"url\":\"$SHARE\",\"password\":\"$NEW_PASSWORD\"}")"
check_eq "A accepts the new password" "200" "$code"
eventually 45 remote_state_is "$REMOTE" connected && pass "A reconnects with the new password" || fail "A reconnects with the new password (state: $(field_a "$REMOTE" .remote_state))"
check_eq "A still lists no password" "null" "$(a "$BASE_A/api/apps" | jq -r --arg id "$REMOTE" '.apps[]|select(.id==$id)|.password')"

hdr "A follows B's application"
b -X POST "$BASE_B/api/apps/$PROBE_APP/stop" >/dev/null
eventually 45 remote_state_is "$REMOTE" stopped && pass "A reports B's app is not running" || fail "A reports B's app is not running (state: $(field_a "$REMOTE" .remote_state))"
check_eq "B's app starts again" "true" "$(b -X POST "$BASE_B/api/apps/$PROBE_APP/start" | jq -r '.ok')"
eventually 45 remote_state_is "$REMOTE" connected && pass "A reconnects after B's app restarts" || fail "A reconnects after B's app restarts (state: $(field_a "$REMOTE" .remote_state))"
check_eq "A stops its remote app" "true" "$(a -X POST "$BASE_A/api/apps/$REMOTE/stop" | jq -r '.ok')"
check_eq "stopped remote app is disconnected" "false|disconnected" "$(field_a "$REMOTE" '"\(.started)|\(.remote_state)"')"
check_eq "B's app keeps running" "running" "$(field_b "$PROBE_APP" .status)"
code="$(b -o /dev/null -w '%{http_code}' -X POST "$BASE_B/api/apps" -d "{\"app_type\":\"remote\",\"url\":\"$SHARE\",\"password\":\"$NEW_PASSWORD\"}")"
check_eq "B cannot map its own application" "400" "$code"

hdr "back to the main desktop"
resp="$(policy "$PROBE_APP" '{"enabled":false}')"
check_eq "disabling disconnects sessions" "true" "$(jq -r '.revoked' <<<"$resp")"
check_eq "disabling a running shared app needs a restart" "true" "$(jq -r '.restart_required' <<<"$resp")"
reset_probe
check_eq "restart accepted" "true" "$(b -X POST "$BASE_B/api/apps/$PROBE_APP/restart" | jq -r '.ok')"
eventually 30 status_is "$PROBE_APP" running && pass "app running in the main desktop again" || fail "app running in the main desktop again"
check_eq "no restart pending" "false|false" "$(field_b "$PROBE_APP" '"\(.sharing.enabled)|\(.sharing.restart_required)"')"
eventually 10 test ! -e "$RUNTIME/endpoint.json" && pass "runtime endpoint gone" || fail "runtime endpoint gone"
probe_sees_host "main desktop again"
code="$(curl -sS -m 15 -o /dev/null -w '%{http_code}' -X POST "$SHARE/authenticate" -H 'Content-Type: application/json' -d "{\"password\":\"$NEW_PASSWORD\"}")"
check_eq "unshared app refuses authentication" "401" "$code"

hdr "authentication limits"
# Run last: the source is locked out of this app for 15 minutes.
policy "$PROBE_APP" "{\"enabled\":true,\"password\":\"$NEW_PASSWORD\"}" >/dev/null
for _ in $(seq 1 8); do
  curl -sS -m 15 -o /dev/null -X POST "$SHARE/authenticate" -H 'Content-Type: application/json' -d '{"password":"wrong-password"}'
done
check_eq "repeated failures are limited" "429" "$(curl -sS -m 15 -o /dev/null -w '%{http_code}' -X POST "$SHARE/authenticate" -H 'Content-Type: application/json' -d "{\"password\":\"$NEW_PASSWORD\"}")"
code="$(curl -sS -m 15 -o /dev/null -w '%{http_code}' -X POST "$BASE_B/remote/v1/apps/$LAUNCHER/authenticate" -H 'Content-Type: application/json' -d '{"password":"wrong-password"}')"
check_eq "other applications are not locked by it" "401" "$code"
[ ! -e "$TMP/b/home/remote-audit.log" ] && pass "no audit log is written" || fail "no audit log is written"

hdr "TLS identity"
out="$(IVNC_HOME="$TMP/tls" HOME="$TMP/tls" XDG_RUNTIME_DIR="$TMP/a/run" IVNC_TLS_CERT_FILE=/nonexistent.pem \
  timeout 8 "$IVNC_BIN" -c "$TMP/a/config.toml" --http-port "$(free_port)" --tls --proxy-autostart false 2>&1)"
grep -q "Set both IVNC_TLS_CERT_FILE and IVNC_TLS_KEY_FILE" <<<"$out" && pass "certificate without key refused" || fail "certificate without key refused"
