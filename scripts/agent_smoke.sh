#!/usr/bin/env bash
#
# agent_smoke.sh — deterministic, secret-free regression smoke suite for the
# built-in VLM agent. Run after every major change.
#
# It boots a fully isolated ivnc instance (own XDG_RUNTIME_DIR / config / data
# dirs, own HTTP port, own Chrome DevTools port, basic-auth disabled) and
# exercises the agent surface WITHOUT any live LLM provider: every agent run
# uses the `replay` provider, so the suite needs no API keys and contacts no
# external network. All output goes to stdout.
#
# What it covers:
#   1. API contract: provider-test endpoint (replay/unknown/error path),
#      agent-config, providers list.
#   2. Schedules CRUD (create / list / delete).
#   3. Input-only replay E2E: mouse_move/type_text/wait/screenshot/done.
#   4. launch_app reuse E2E: a 2nd launch_app(chrome, url) on a running Chrome
#      must deliver the URL via DevTools (the reuse fix) instead of dropping it.
#
# Env knobs (all optional):
#   IVNC_BIN=path      binary to test          (default: target/release/ivnc)
#   SKIP_BUILD=1       skip `cargo build`      (default: build if not SKIP_BUILD)
#   SKIP_UNIT=1        skip `cargo test`       (default: run if not SKIP_UNIT)
#   SKIP_CHROME=1      skip the launch_app E2E (default: run it)
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

# check NAME EXPECTED ACTUAL — equality assertion
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

free_port() {
  python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()'
}

# ----------------------------------------------------------------------------
# Optional build + unit tests.
# ----------------------------------------------------------------------------
if [ "${SKIP_BUILD:-}" != "1" ]; then
  hdr "cargo build --release --features agent-all"
  if cargo build --release --features agent-all; then
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
  hdr "cargo test --features agent-all"
  if cargo test --features agent-all; then
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
  # Stop Chrome via API (best effort), then any chrome bound to OUR debug port only.
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

# Wait for HTTP to come up.
READY=0
for _ in $(seq 1 60); do
  if ! kill -0 "$IVNC_PID" 2>/dev/null; then
    say "ERROR: ivnc exited during startup — last log lines:"
    tail -n 30 "$TMP/ivnc.log" || true
    fail "ivnc boot"
    exit 1
  fi
  if curl -sS -m 2 "$BASE/api/console/providers" >/dev/null 2>&1; then
    READY=1; break
  fi
  sleep 0.5
done
if [ "$READY" = "1" ]; then pass "ivnc boot (HTTP ready)"; else
  fail "ivnc boot (HTTP never became ready)"; tail -n 30 "$TMP/ivnc.log" || true; exit 1
fi

# ----------------------------------------------------------------------------
# 1. API contract checks.
# ----------------------------------------------------------------------------
hdr "API: provider test endpoint"

# replay → ok + skipped, no network.
resp="$(curl -sS -m 10 -X POST "$BASE/api/console/providers/replay/test" \
  -H 'content-type: application/json' -d '{}')"
check_eq  "replay test ok"      "true" "$(jq -r '.ok // empty' <<<"$resp")"
check_eq  "replay test skipped" "true" "$(jq -r '.skipped // empty' <<<"$resp")"

# unknown provider → HTTP 404.
code="$(curl -sS -m 10 -o /dev/null -w '%{http_code}' -X POST \
  "$BASE/api/console/providers/__nope__/test" -H 'content-type: application/json' -d '{}')"
check_eq  "unknown provider 404" "404" "$code"

# openai test against an unreachable local endpoint → HTTP 200, ok:false (error path,
# no external network, no secret).
resp="$(curl -sS -m 20 -X POST "$BASE/api/console/providers/openai/test" \
  -H 'content-type: application/json' \
  -d '{"endpoint":"http://127.0.0.1:1/v1","api_key":"x","clear_api_key":false}')"
check_eq  "openai probe error-path ok=false" "false" "$(jq -r 'if has("ok") then .ok else "MISSING" end' <<<"$resp")"

hdr "API: providers list + agent-config"
resp="$(curl -sS -m 10 "$BASE/api/console/providers")"
check_contains "providers list has replay" '"name":"replay"' "$(jq -c '.providers' <<<"$resp")"
code="$(curl -sS -m 10 -o /dev/null -w '%{http_code}' "$BASE/api/console/agent-config")"
check_eq  "agent-config 200" "200" "$code"

hdr "API: apps list has builtin-chrome"
resp="$(curl -sS -m 10 "$BASE/api/apps")"
check_contains "apps list has builtin-chrome" '"id":"builtin-chrome"' "$(jq -c '.apps' <<<"$resp")"

# ----------------------------------------------------------------------------
# 2. Schedules CRUD.
# ----------------------------------------------------------------------------
hdr "API: schedules CRUD"
sched_body='{"name":"smoke-sched","cron":"0 9 * * *","task":"noop","provider":"replay"}'
resp="$(curl -sS -m 10 -X POST "$BASE/api/console/schedules" \
  -H 'content-type: application/json' -d "$sched_body")"
SCHED_ID="$(jq -r '.schedule.id // empty' <<<"$resp")"
check_eq  "schedule create ok" "true" "$(jq -r '.ok // empty' <<<"$resp")"
if [ -n "$SCHED_ID" ]; then pass "schedule create returned id ($SCHED_ID)"; else fail "schedule create returned id"; fi

resp="$(curl -sS -m 10 "$BASE/api/console/schedules")"
check_contains "schedule appears in list" "$SCHED_ID" "$(jq -c '.schedules' <<<"$resp")"

if [ -n "$SCHED_ID" ]; then
  resp="$(curl -sS -m 10 -X DELETE "$BASE/api/console/schedules/$SCHED_ID")"
  check_eq  "schedule delete ok" "true" "$(jq -r '.ok // empty' <<<"$resp")"
  resp="$(curl -sS -m 10 "$BASE/api/console/schedules")"
  if jq -e --arg id "$SCHED_ID" '.schedules | any(.id == $id)' <<<"$resp" >/dev/null 2>&1; then
    fail "schedule gone after delete"
  else
    pass "schedule gone after delete"
  fi
fi

# ----------------------------------------------------------------------------
# Agent-run helpers (replay provider).
# ----------------------------------------------------------------------------
# start_replay TASK ACTIONS_JSON -> echoes run_id (empty on failure)
start_replay() {
  local task="$1" actions="$2" body resp
  body="$(jq -nc --arg task "$task" --argjson actions "$actions" \
    '{task:$task, provider:"replay", replay_actions:$actions,
      budget:{max_steps:50, max_wall_seconds:120},
      options:{action_settle_ms:50, record_trajectory:true}}')"
  resp="$(curl -sS -m 10 -X POST "$BASE/api/console/agent-start" \
    -H 'content-type: application/json' -d "$body")"
  jq -r '.run_id // empty' <<<"$resp"
}

# wait_run RUN_ID -> echoes final steps-endpoint JSON (report+steps); empty on timeout
wait_run() {
  local run_id="$1" resp reason
  for _ in $(seq 1 80); do
    resp="$(curl -sS -m 10 "$BASE/api/console/agent-runs/$run_id/steps")"
    reason="$(jq -r '.report.finish_reason.kind // "running"' <<<"$resp")"
    if [ "$reason" != "running" ]; then
      printf '%s' "$resp"; return 0
    fi
    sleep 0.25
  done
  printf '%s' "$resp"
  return 1
}

# ----------------------------------------------------------------------------
# 3. Input-only replay E2E.
# ----------------------------------------------------------------------------
hdr "E2E: input-only replay run"
actions='[
  {"kind":"mouse_move","x":10,"y":10},
  {"kind":"type_text","text":"smoke-test","press_enter":false},
  {"kind":"wait","ms":50},
  {"kind":"screenshot"},
  {"kind":"done","success":true,"reason":"smoke complete","output":"ok"}
]'
RUN_ID="$(start_replay "smoke input replay" "$actions")"
if [ -n "$RUN_ID" ]; then
  pass "input replay started ($RUN_ID)"
  if result="$(wait_run "$RUN_ID")"; then
    check_eq "input replay finished done"    "done" "$(jq -r '.report.finish_reason.kind' <<<"$result")"
    check_eq "input replay success"          "true" "$(jq -r '.report.finish_reason.success // .report.success' <<<"$result")"
    # screenshot step executed with result ok
    if jq -e '.steps | any(.action.kind == "screenshot" and .result.kind == "ok")' <<<"$result" >/dev/null 2>&1; then
      pass "screenshot step result ok"
    else
      fail "screenshot step result ok"
    fi
    # type_text step executed ok
    if jq -e '.steps | any(.action.kind == "type_text" and .result.kind == "ok")' <<<"$result" >/dev/null 2>&1; then
      pass "type_text step result ok"
    else
      fail "type_text step result ok"
    fi
  else
    fail "input replay finished (timed out waiting)"
  fi
else
  fail "input replay started"
fi

# ----------------------------------------------------------------------------
# 4. launch_app reuse E2E (verifies the DevTools URL-delivery fix).
# ----------------------------------------------------------------------------
if [ "${SKIP_CHROME:-}" = "1" ]; then
  hdr "E2E: launch_app reuse — SKIPPED (SKIP_CHROME=1)"
else
  hdr "E2E: launch_app reuse (Chrome DevTools URL delivery)"
  # Unreachable but syntactically valid marker URLs: Chrome creates the tabs
  # regardless of whether they load, and we never touch the external network.
  URL_A="http://127.0.0.1:9/marker-aaa"
  URL_B="http://127.0.0.1:9/marker-bbb"

  # Run A: launch Chrome with the first URL (delivered as a CLI arg).
  actions_a="$(jq -nc --arg u "$URL_A" \
    '[{"kind":"launch_app","app":"chrome","url":$u},
      {"kind":"done","success":true,"reason":"launched","output":""}]')"
  RUN_A="$(start_replay "launch chrome A" "$actions_a")"
  if [ -n "$RUN_A" ]; then
    pass "launch_app run A started ($RUN_A)"
    res_a="$(wait_run "$RUN_A")" || true
    a_launch_kind="$(jq -r '.steps[] | select(.action.kind=="launch_app") | .result.kind' <<<"$res_a" | head -n1)"
    check_eq "run A launch_app result ok" "ok" "${a_launch_kind:-MISSING}"
  else
    fail "launch_app run A started"
  fi

  # Poll OUR Chrome's DevTools port until it answers (cold start can take a while).
  CHROME_UP=0
  for _ in $(seq 1 120); do
    if curl -sS -m 2 "http://127.0.0.1:$DEBUG_PORT/json/version" >/dev/null 2>&1; then
      CHROME_UP=1; break
    fi
    sleep 0.5
  done
  if [ "$CHROME_UP" = "1" ]; then
    pass "Chrome DevTools reachable on :$DEBUG_PORT"
  else
    fail "Chrome DevTools reachable on :$DEBUG_PORT (cold start timed out; see $TMP/ivnc.log)"
  fi

  if [ "$CHROME_UP" = "1" ]; then
    pages_before="$(curl -sS -m 5 "http://127.0.0.1:$DEBUG_PORT/json/list" \
      | jq '[.[] | select(.type=="page")] | length' 2>/dev/null || echo 0)"
    say "  page targets before run B: $pages_before"

    # Run B: Chrome already running → 2nd launch_app(url) must hand the URL to
    # the running browser via DevTools (OpenedInExisting), NOT drop it.
    actions_b="$(jq -nc --arg u "$URL_B" \
      '[{"kind":"launch_app","app":"chrome","url":$u},
        {"kind":"done","success":true,"reason":"reused","output":""}]')"
    RUN_B="$(start_replay "launch chrome B reuse" "$actions_b")"
    if [ -n "$RUN_B" ]; then
      pass "launch_app run B started ($RUN_B)"
      res_b="$(wait_run "$RUN_B")" || true
      b_step="$(jq -c '.steps[] | select(.action.kind=="launch_app")' <<<"$res_b" | head -n1)"
      b_kind="$(jq -r '.result.kind' <<<"$b_step" 2>/dev/null)"
      b_msg="$(jq -r '.result.message // ""' <<<"$b_step" 2>/dev/null)"
      # The fix: result must be ok, NOT executor_error/launch_app_no_op.
      check_eq "run B launch_app result ok (URL delivered)" "ok" "${b_kind:-MISSING}"
      case "$b_msg" in
        *launch_app_no_op*) fail "run B did NOT drop the URL (got launch_app_no_op)";;
        *) pass "run B did NOT report launch_app_no_op";;
      esac

      # Give DevTools a moment to register the new tab, then assert the tab count grew.
      sleep 1
      pages_after="$(curl -sS -m 5 "http://127.0.0.1:$DEBUG_PORT/json/list" \
        | jq '[.[] | select(.type=="page")] | length' 2>/dev/null || echo 0)"
      say "  page targets after run B: $pages_after"
      if [ "${pages_after:-0}" -ge 2 ] && [ "${pages_after:-0}" -gt "${pages_before:-0}" ]; then
        pass "DevTools shows an extra tab after reuse ($pages_before -> $pages_after)"
      else
        fail "DevTools shows an extra tab after reuse ($pages_before -> $pages_after)"
      fi
    else
      fail "launch_app run B started"
    fi
  fi
fi

# ----------------------------------------------------------------------------
# Summary.
# ----------------------------------------------------------------------------
hdr "summary"
say "PASS: $PASS"
say "FAIL: $FAIL"
if [ "$FAIL" -gt 0 ]; then
  say "failed checks:"
  for n in "${FAILED_NAMES[@]}"; do say "  - $n"; done
  exit 1
fi
say "all checks passed"
exit 0
