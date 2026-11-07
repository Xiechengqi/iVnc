#!/usr/bin/env bash
# End-to-end checks for application runtime isolation levels and remote access
# management. Boots two temporary instances (A and B); never touches running ones.
#
#   1. Host isolation sees host files, host loopback and the manager's locale;
#      it cannot be shared.
#   2. Strict isolation sees only approved host paths, read-only unless marked
#      writable; credential and system paths are refused.
#   3. A launcher that forks and exits keeps its application running.
#   4. A crashing application is restarted with backoff, not in a tight loop.
#   5. Runtime settings keep saved credentials; revoking one connection on B
#      ends it and A reports that the password is required.
#   6. Failed authentication is limited per source; mapping B onto itself is refused.
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

for tool in bwrap slirp4netns nft ip jq curl python3; do
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

hdr "boot"
start_instance b "$PORT_B" true
start_instance a "$PORT_A" false
eventually 30 curl -sf "$BASE_B/health" && eventually 30 curl -sf "$BASE_A/health" \
  && pass "instances healthy" || { fail "instances healthy"; tail -20 "$TMP/b/server.log"; exit 1; }

SHARED="$TMP/host-data"
mkdir -p "$SHARED"
echo "visible" > "$SHARED/marker"

hdr "host isolation"
PROBE='cat '"$SHARED"'/marker > "$HOME/seen" 2>&1; echo "$TZ|$LANG|${SMOKE_SECRET:-}" > "$HOME/env"; curl -s -m 3 -o /dev/null -w "%{http_code}" '"$BASE_B"'/health > "$HOME/net" 2>&1; exec sleep 600'
HOST_APP="$(add_app host-probe "$PROBE")"
check_eq "host mode saved" "host" "$(policy "$HOST_APP" '{"enabled":false,"desktop":{"isolation":"host"}}' | jq -r '.policy.desktop.isolation')"
check_eq "host mode cannot be shared" "Shared applications must use strict isolation" \
  "$(policy "$HOST_APP" "{\"enabled\":true,\"password\":\"$APP_PASSWORD\"}" | jq -r '.error')"
check_eq "host app starts" "true" "$(b -X POST "$BASE_B/api/apps/$HOST_APP/start" | jq -r '.ok')"
eventually 15 test -s "$(home_of "$HOST_APP")/net"
check_eq "host app reads host files" "visible" "$(cat "$(home_of "$HOST_APP")/seen" 2>/dev/null)"
check_eq "host app reaches host loopback" "200" "$(cat "$(home_of "$HOST_APP")/net" 2>/dev/null)"
check_eq "host app inherits manager environment" "Asia/Shanghai|C.UTF-8|manager-only" "$(cat "$(home_of "$HOST_APP")/env" 2>/dev/null)"

hdr "strict isolation with approved paths"
STRICT_APP="$(add_app strict-probe 'cat '"$SHARED"'/marker > "$HOME/seen" 2>&1; (touch '"$SHARED"'/written 2>/dev/null && echo writable || echo readonly) > "$HOME/write"; echo "$TZ|$LANG|${SMOKE_SECRET:-}" > "$HOME/env"; curl -s -m 3 -o /dev/null -w "%{http_code}" '"$BASE_B"'/health > "$HOME/net" 2>&1; exec sleep 600')"
code="$(b -o /dev/null -w '%{http_code}' -X PUT "$BASE_B/api/apps/$STRICT_APP/sharing" -d '{"enabled":false,"desktop":{"mounts":[{"path":"/usr/share","writable":true}]}}')"
check_eq "writable system path refused" "400" "$code"
code="$(b -o /dev/null -w '%{http_code}' -X PUT "$BASE_B/api/apps/$STRICT_APP/sharing" -d "{\"enabled\":false,\"desktop\":{\"mounts\":[{\"path\":\"$TMP/b\"}]}}")"
check_eq "path containing iVnc data refused" "400" "$code"
code="$(b -o /dev/null -w '%{http_code}' -X PUT "$BASE_B/api/apps/$STRICT_APP/sharing" -d '{"enabled":false,"desktop":{"networks":["127.0.0.0/8"]}}')"
check_eq "loopback network refused" "400" "$code"
resp="$(policy "$STRICT_APP" "{\"enabled\":false,\"desktop\":{\"mounts\":[{\"path\":\"$SHARED\"}],\"networks\":[\"192.168.250.0/24\"]}}")"
check_eq "approved path saved" "$SHARED" "$(jq -r '.policy.desktop.mounts[0].path' <<<"$resp")"
check_eq "strict app starts" "true" "$(b -X POST "$BASE_B/api/apps/$STRICT_APP/start" | jq -r '.ok')"
eventually 15 test -s "$(home_of "$STRICT_APP")/net"
check_eq "approved path is visible" "visible" "$(cat "$(home_of "$STRICT_APP")/seen" 2>/dev/null)"
check_eq "approved path is read-only" "readonly" "$(cat "$(home_of "$STRICT_APP")/write" 2>/dev/null)"
check_eq "strict app keeps locale only" "Asia/Shanghai|C.UTF-8|" "$(cat "$(home_of "$STRICT_APP")/env" 2>/dev/null)"
net="$(cat "$(home_of "$STRICT_APP")/net" 2>/dev/null)"
[ "$net" != "200" ] && pass "strict app cannot reach host loopback ($net)" || fail "strict app reached host loopback"

hdr "launcher that forks and exits"
LAUNCHER="$(add_app launcher '(sleep 600 &) ; exit 0')"
check_eq "launcher app starts" "true" "$(b -X POST "$BASE_B/api/apps/$LAUNCHER/start" | jq -r '.ok')"
sleep 6
check_eq "forked application keeps running" "running" "$(b "$BASE_B/api/apps/$LAUNCHER" | jq -r '.app.status // .status')"
b -X POST "$BASE_B/api/apps/$LAUNCHER/stop" >/dev/null
sleep 1
check_eq "stopped launcher app" "stopped" "$(b "$BASE_B/api/apps/$LAUNCHER" | jq -r '.app.status // .status')"

hdr "crash backoff"
CODE="$(b -o /dev/null -w '%{http_code}' -X POST "$BASE_B/api/apps/$(add_app fails-at-start 'exit 3')/start")"
check_eq "failed start is reported, not retried" "500" "$CODE"
CRASHER="$(add_app crasher 'sleep 4; exit 3')"
check_eq "crashing app starts" "true" "$(b -X POST "$BASE_B/api/apps/$CRASHER/start" | jq -r '.ok')"
sleep 20
starts="$(grep -c "restarted app 'crasher'" "$TMP/b/server.log")"
[ "$starts" -ge 1 ] && [ "$starts" -le 2 ] && pass "crashing app restarted with backoff ($starts restarts in 20s)" || fail "crash loop: $starts restarts in 20s"
grep -q "app $CRASHER exited unexpectedly; restarting in" "$TMP/b/server.log" && pass "backoff delay logged" || fail "backoff delay logged"
check_eq "crashing app reported" "crashed" "$(b "$BASE_B/api/apps/$CRASHER" | jq -r '.app.status // .status')"
b -X POST "$BASE_B/api/apps/$CRASHER/stop" >/dev/null

hdr "credentials and runtime settings"
policy "$STRICT_APP" "{\"enabled\":true,\"password\":\"$APP_PASSWORD\"}" >/dev/null
SHARE="$BASE_B/remote/v1/apps/$STRICT_APP"
cred="$(curl -sS -m 15 -X POST "$SHARE/authenticate" -H 'Content-Type: application/json' \
  -d "{\"password\":\"$APP_PASSWORD\",\"client\":{\"node_id\":\"n1\",\"name\":\"smoke-a\"}}" | jq -r '.credential')"
check_eq "credential issued" "64" "${#cred}"
resp="$(policy "$STRICT_APP" '{"enabled":true,"desktop":{"x11":true,"mounts":[],"networks":[]}}')"
check_eq "runtime change keeps credentials" "false" "$(jq -r '.revoked' <<<"$resp")"
check_eq "restart required after runtime change" "true" "$(jq -r '.restart_required' <<<"$resp")"
check_eq "credential still valid" "200" "$(curl -sS -m 15 -o /dev/null -w '%{http_code}' "$SHARE/info" -H "Authorization: Bearer $cred")"
check_eq "B reports not ready until restart" "false" "$(curl -sS -m 15 "$SHARE/info" -H "Authorization: Bearer $cred" | jq -r '.ready')"
grants="$(b "$BASE_B/api/apps/$STRICT_APP/sharing")"
check_eq "connection lists the reporting instance" "smoke-a" "$(jq -r '.grants[0].label' <<<"$grants")"
check_eq "connection lists the source address" "127.0.0.1" "$(jq -r '.grants[0].ip' <<<"$grants")"
policy "$STRICT_APP" '{"enabled":true,"desktop":{"mounts":[{"path":"'"$SHARED"'"}],"networks":["192.168.250.0/24"]}}' >/dev/null

hdr "remote app on A"
resp="$(a -X POST "$BASE_A/api/apps" -d "{\"app_type\":\"remote\",\"url\":\"$SHARE\",\"password\":\"$APP_PASSWORD\"}")"
REMOTE="$(jq -r '.app.id' <<<"$resp")"
[ -n "$REMOTE" ] && [ "$REMOTE" != null ] && pass "remote app added on A" || fail "remote app added on A: $resp"
a -X POST "$BASE_A/api/apps/$REMOTE/view" >/dev/null
check_eq "A caches B state" "running" "$(a "$BASE_A/api/apps" | jq -r --arg id "$REMOTE" '.apps[]|select(.id==$id)|.remote_state')"
grant="$(b "$BASE_B/api/apps/$STRICT_APP/sharing" | jq -r '.grants[] | select(.label!="smoke-a") | .id' | head -1)"
check_eq "B revokes one connection" "true" "$(b -X DELETE "$BASE_B/api/apps/$STRICT_APP/grants/$grant" | jq -r '.ok')"
check_eq "other connection unaffected" "200" "$(curl -sS -m 15 -o /dev/null -w '%{http_code}' "$SHARE/info" -H "Authorization: Bearer $cred")"
code="$(a -o /dev/null -w '%{http_code}' -X POST "$BASE_A/api/apps/$REMOTE/view")"
check_eq "revoked view refused" "502" "$code"
check_eq "A asks for the password again" "auth_required" "$(a "$BASE_A/api/apps" | jq -r --arg id "$REMOTE" '.apps[]|select(.id==$id)|.remote_state')"
code="$(b -o /dev/null -w '%{http_code}' -X POST "$BASE_B/api/apps" -d "{\"app_type\":\"remote\",\"url\":\"$SHARE\",\"password\":\"$APP_PASSWORD\"}")"
check_eq "B cannot map its own application" "400" "$code"

hdr "authentication limits"
for _ in $(seq 1 8); do
  curl -sS -m 15 -o /dev/null -X POST "$SHARE/authenticate" -H 'Content-Type: application/json' -d '{"password":"wrong-password"}'
done
check_eq "repeated failures are limited" "429" "$(curl -sS -m 15 -o /dev/null -w '%{http_code}' -X POST "$SHARE/authenticate" -H 'Content-Type: application/json' -d "{\"password\":\"$APP_PASSWORD\"}")"
code="$(curl -sS -m 15 -o /dev/null -w '%{http_code}' -X POST "$BASE_B/remote/v1/apps/$HOST_APP/authenticate" -H 'Content-Type: application/json' -d '{"password":"wrong-password"}')"
check_eq "other applications are not locked by it" "401" "$code"
grep -q '"event":"auth_failure"' "$TMP/b/home/remote-audit.log" && pass "failures audited" || fail "failures audited"
check_eq "audit log is private" "600" "$(stat -c '%a' "$TMP/b/home/remote-audit.log")"

hdr "TLS identity"
out="$(IVNC_HOME="$TMP/tls" HOME="$TMP/tls" XDG_RUNTIME_DIR="$TMP/a/run" IVNC_TLS_CERT_FILE=/nonexistent.pem \
  timeout 8 "$IVNC_BIN" -c "$TMP/a/config.toml" --http-port "$(free_port)" --tls --proxy-autostart false 2>&1)"
grep -q "Set both IVNC_TLS_CERT_FILE and IVNC_TLS_KEY_FILE" <<<"$out" && pass "certificate without key refused" || fail "certificate without key refused"
