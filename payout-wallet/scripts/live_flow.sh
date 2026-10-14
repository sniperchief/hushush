#!/usr/bin/env bash
# hushush LIVE TESTNET run of the Phase 5 flow.
#
# Real everything except the passkey authenticator (a SOFTWARE one, see
# web/scripts/lib/soft-passkey.mjs): the Next.js app (`next start`),
# payroll-api, PostgreSQL, the payout wallet service on the `treasury` wallet,
# lightwalletd on the NU7 testnet. The worker's payout address is `carol`'s
# (Phase 0 CLI wallet), which receives the withdrawals.
#
#   1. employer signs up (passkey), names the company, creates a stream
#      (LIVE_TOTAL over LIVE_SECS) and gets the worker link
#   2. worker opens the link, signs up (passkey), joins; an invalid address is
#      refused by the real wallet; carol's address is locked; a change is refused
#   3. Withdraw before the first milestone is refused
#   4. at 20%: 6 CONCURRENT taps (3 sharing a key, 3 with new keys) → exactly
#      one withdrawal, one transaction
#   5. the employer ends the stream early → the remainder (< 20%) unlocks
#   6. first withdrawal confirmed (LIVE confirmations) → remainder withdrawn →
#      confirmed
#   Then: carol's wallet shows each withdrawal exactly once with its memo; the
#   treasury holds exactly one transaction per payout memo; treasury and carol
#   accounting; server-rendered pages.
#
# Prerequisites: as live_e2e.sh (docs/PHASE3_PAYOUTS.md §7) — funded
# `treasury`, `carol`, `cargo build` in payroll-api and payout-wallet,
# `npm run build` in web, dev PostgreSQL and secrets in %LOCALAPPDATA%\hushush-dev.
# Ports 3300 (web), 7980 (payroll-api) and 7979 (wallet) must be free.
# TESTNET ONLY.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
D="$LOCALAPPDATA/hushush-dev"
RUN_ID=${RUN_ID:-p5-$(date -u +%Y%m%d%H%M%S)}
EVID="$ROOT/payout-wallet/evidence"
LOG="$EVID/live-$RUN_ID.log"
SVC="$EVID/live-$RUN_ID.services.log"
CLI="$ROOT/wallet-proof/target/debug/hushush-wallet-proof.exe"
WALLET_BIN="$ROOT/payout-wallet/target/debug/hushush-payout-wallet.exe"
API_BIN="$ROOT/payroll-api/target/debug/hushush-payroll-api.exe"
LWD=${WALLET_LIGHTWALLETD_URL:-https://testnet.zec.rocks:443}
LIVE_TOTAL=${LIVE_TOTAL:-0.001}
LIVE_SECS=${LIVE_SECS:-600}
CONF=${PAYOUT_CONFIRMATIONS:-10}
mkdir -p "$EVID"
export PYTHONUTF8=1 NO_COLOR=1 RUST_LOG=${RUST_LOG:-info,zcash_client_backend=warn}

log() { echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$LOG"; }
die() { log "FAIL: $*"; exit 1; }
jget() { python -c "import sys,json;d=json.load(sys.stdin);print($1)"; }

set -a; . "$D/dev.env"; set +a
DB_NAME="hushush_live_${RUN_ID//-/_}"
DB_URL="${DATABASE_URL%/*}/$DB_NAME"
# Not exported: only the wallet and payroll-api receive it.
WALLET_API_TOKEN=$(cat "$D/wallet-api-token.txt")
PAYROLL_API_TOKEN=$(python -c "import secrets;print(secrets.token_hex(32))")
PAYOUT_ADDRESS_KEY=$(python -c "import secrets;print(secrets.token_hex(32))")
export APP_URL=http://localhost:3300 STATE_FILE="$D/live-state-$RUN_ID.json"
LF() { (cd "$ROOT/web" && node scripts/live-flow.mjs "$@"); }
winpid() { cat "/proc/$1/winpid"; }
hard_kill() { taskkill //F //T //PID "$(winpid "$1")" >/dev/null 2>&1 || kill -9 "$1" 2>/dev/null || true; }
wait_http() { for _ in $(seq 1 "${2:-180}"); do curl -sf -o /dev/null "$1" && return 0; sleep 1; done; die "$1 never came up"; }
wstatus() { curl -s -H "Authorization: Bearer $WALLET_API_TOKEN" http://127.0.0.1:7979/v1/status; }
port_pid() { netstat -ano | awk -v p=":$1" '$2 ~ p"$" && $4 == "LISTENING" {print $5; exit}'; }
cleanup() {
  for p in ${WEB_PID:-} ${API_PID:-} ${WALLET_PID:-} ${AWAKE_PID:-}; do hard_kill "$p"; done
  for port in 3300 7980 7979; do
    w=$(port_pid $port); [ -n "$w" ] && taskkill //F //T //PID "$w" >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT
memo_count() { # wallet name, payout id -> number of transactions carrying that payout's memo
  "$CLI" history "$1" | python -c "
import sys, json
h = json.load(sys.stdin); memo = 'hushush payout ' + sys.argv[1]
print(len({t['txid'] for t in h for o in t['outputs'] if o['memo'] == memo}))" "$2"
}

for port in 3300 7980 7979; do
  [ -z "$(port_pid $port)" ] || { echo "port $port is already in use; refusing to start" >&2; exit 1; }
done
powershell -NoProfile -Command "Add-Type -Name P -Namespace W -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint f);'; [void][W.P]::SetThreadExecutionState(0x80000001); while (\$true) { Start-Sleep 60 }" >/dev/null 2>&1 &
AWAKE_PID=$!

log "=== hushush Phase 5 live flow run $RUN_ID (TESTNET, $LWD, $CONF confirmations, stream $LIVE_TOTAL ZEC over ${LIVE_SECS}s)"
log "lightwalletd: $("$CLI" info)"
(cd "$ROOT/web" && node -e "
const pg=require('pg');const c=new pg.Client({connectionString:process.env.DATABASE_URL});
c.connect().then(()=>c.query('CREATE DATABASE $DB_NAME')).then(()=>c.end())")
log "database $DB_NAME created"

WALLET_API_TOKEN="$WALLET_API_TOKEN" WALLET_NAME=treasury WALLET_LIGHTWALLETD_URL="$LWD" WALLET_SYNC_SECS=20 "$WALLET_BIN" >>"$SVC" 2>&1 &
WALLET_PID=$!
wait_http http://127.0.0.1:7979/healthz 600
PAYROLL_DATABASE_URL="$DB_URL" PAYROLL_API_ADDR=127.0.0.1:7980 PAYROLL_API_TOKEN="$PAYROLL_API_TOKEN" \
WALLET_URL=http://127.0.0.1:7979 WALLET_API_TOKEN="$WALLET_API_TOKEN" PAYOUT_ENABLED_ORGS='*' PAYOUT_CONFIRMATIONS="$CONF" \
PAYOUT_ADDRESS_KEY="$PAYOUT_ADDRESS_KEY" PAYOUT_POLL_SECS=15 WALLET_SUBMIT_TIMEOUT_SECS=180 "$API_BIN" >>"$SVC" 2>&1 &
API_PID=$!
wait_http http://127.0.0.1:7980/healthz
(cd "$ROOT/web" && DATABASE_URL="$DB_URL" npm run db:migrate >/dev/null && DATABASE_URL="$DB_URL" PAYROLL_API_URL=http://127.0.0.1:7980 PAYROLL_API_TOKEN="$PAYROLL_API_TOKEN" npx next start -p 3300 >>"$SVC" 2>&1) &
WEB_PID=$!
wait_http http://localhost:3300/login
log "services up: wallet pid $WALLET_PID, payroll-api pid $API_PID, web pid $WEB_PID"

CAROL=$("$CLI" address carol)
ALICE=$("$CLI" address alice)
CAROL_BEFORE=$("$CLI" sync carol 2>/dev/null | jget "d['total']")
TREAS0=$(wstatus | jget "d['total_zats']")
log "treasury total=$TREAS0 spendable=$(wstatus | jget "d['spendable_zats']"); carol total=$CAROL_BEFORE"

log "=== 1-2: employer, stream, worker link, worker joins"
log "setup: $(LF setup "$LIVE_TOTAL" "$LIVE_SECS")"
R=$(LF address utest1thisisnotarealunifiedaddressatall); log "invalid address: $R"
[ "$(echo "$R" | jget "d['status']")" = 422 ] || die "the real wallet did not refuse an invalid address"
R=$(LF address "$CAROL"); log "carol's address: $R"
[ "$(echo "$R" | jget "d['address_locked']")" = True ] || die "address not locked"
R=$(LF address "$ALICE"); log "change to another address: $R"
[ "$(echo "$R" | jget "d['status']")" = 409 ] || die "the address lock did not hold"

log "=== 3: before the first milestone"
R=$(LF withdraw 1); log "early withdraw: $R"
[ "$(echo "$R" | jget "d['statuses']")" = "[409]" ] || die "withdraw before 20% was not refused"

log "=== 4: at 20%, six concurrent taps"
log "milestone: $(LF wait-withdrawable $((LIVE_SECS + 60)))"
R=$(LF withdraw 6); log "6 concurrent taps: $R"
echo "$R" | python -c "
import json,sys; r=json.load(sys.stdin)
assert r['created'] == 1 and len(r['payout_ids']) == 1 and all(s in (200, 201) for s in r['statuses']), r
print('one withdrawal created; the other five got the same withdrawal back')" | tee -a "$LOG"
P1=$(echo "$R" | jget "d['payout_ids'][0]"); A1=$(echo "$R" | jget "d['amount_zats']")
log "withdrawal 1: payout $P1, $A1 zats"

log "=== 5: the employer ends the stream early"
sleep 20
R=$(LF end); log "end: $R"
REM=$(echo "$R" | jget "d['available_zats']")
python -c "import sys; t=round(float('$LIVE_TOTAL')*1e8); r=int('$REM'); assert 0 < r*5 < t, (r, t)" || die "remainder is not between 0 and 20%"
R=$(LF withdraw 1); log "remainder withdraw while #1 is unfinished: $R"
[ "$(echo "$R" | jget "d['payout_ids']")" = "['$P1']" ] || die "a second withdrawal started while the first was unfinished"

log "=== 6: confirmations, then the remainder"
LF wait "$P1" confirmed 3000 | tee -a "$LOG"
R=$(LF withdraw 2); log "remainder (< 20%) after the end, 2 taps: $R"
echo "$R" | python -c "
import json,sys; r=json.load(sys.stdin); assert r['created'] == 1 and len(r['payout_ids']) == 1, r" || die "remainder withdrawal wrong"
P2=$(echo "$R" | jget "d['payout_ids'][0]"); A2=$(echo "$R" | jget "d['amount_zats']")
[ "$A2" = "$REM" ] || die "remainder paid $A2, expected $REM"
LF wait "$P2" confirmed 3000 | tee -a "$LOG"

log "=== verification"
S=$(LF show); log "stream: $S"
echo "$S" | python -c "
import json,sys; s=json.load(sys.stdin); b=s['balance']
assert s['status'] == 'ended' and b['available_zats'] == 0 and b['reserved_zats'] == 0, s
assert b['claimed_zats'] == b['earned_zats'] == $A1 + $A2, s
assert [p['state'] for p in s['payouts']] == ['confirmed', 'confirmed'], s
print('everything earned was withdrawn, in two confirmed payouts')" | tee -a "$LOG"
"$CLI" sync carol >/dev/null 2>&1 || "$CLI" sync carol >/dev/null
TOTAL_FEES=0
TXIDS=()
for PID_ in "$P2" "$P1"; do
  P=$(echo "$S" | python -c "import json,sys; print(json.dumps([p for p in json.load(sys.stdin)['payouts'] if p['id'] == '$PID_'][0]))")
  TX=$(echo "$P" | jget "d['txid']"); FEE=$(echo "$P" | jget "d['fee_zats']"); AMT=$(echo "$P" | jget "d['amount_zats']")
  TOTAL_FEES=$((TOTAL_FEES + FEE)); TXIDS+=("$TX")
  "$CLI" history carol | python -c "
import sys, json
h = json.load(sys.stdin); memo = 'hushush payout ' + sys.argv[1]
hits = [(t['txid'], o['value_zats']) for t in h for o in t['outputs'] if not o['is_sent'] and o['memo'] == memo]
assert hits == [(sys.argv[2], int(sys.argv[3]))], hits
print('carol received exactly once:', hits[0][1], 'zats in', sys.argv[2], 'memo', repr(memo))
" "$PID_" "$TX" "$AMT" | tee -a "$LOG"
  N_TX=$(memo_count treasury "$PID_"); log "treasury transactions carrying payout $PID_'s memo: $N_TX"
  [ "$N_TX" = 1 ] || die "payout $PID_ has $N_TX transactions"
done
CAROL_AFTER=$("$CLI" balance carol | jget "d['total']")
log "carol total $CAROL_BEFORE → $CAROL_AFTER (delta $((CAROL_AFTER - CAROL_BEFORE)), expected $((A1 + A2)))"
[ $((CAROL_AFTER - CAROL_BEFORE)) -eq $((A1 + A2)) ] || die "carol delta wrong"
sleep 25
TREAS1=$(wstatus | jget "d['total_zats']")
log "treasury total $TREAS0 → $TREAS1 (delta $((TREAS0 - TREAS1)), expected $((A1 + A2 + TOTAL_FEES)) = withdrawals + fees)"
[ $((TREAS0 - TREAS1)) -eq $((A1 + A2 + TOTAL_FEES)) ] || die "treasury accounting wrong"
log "pages: $(LF pages "$CAROL" "${TXIDS[@]}")"
log "paid txids: ${TXIDS[*]}"
log "PASS: Phase 5 flow live on testnet — passkey accounts, worker link, locked address, milestone + remainder withdrawals, exactly once"
