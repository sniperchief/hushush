#!/usr/bin/env bash
# hushush LIVE TESTNET end-to-end payout run (Phase 3, extended in Phase 4).
#
# Real everything: the Next.js app (`next start`), payroll-api, PostgreSQL,
# the payout wallet service on the `treasury` wallet, lightwalletd on the NU7
# testnet. A contributor's wallet (`carol`, Phase 0 CLI) receives the payouts.
#
#   A. wages visible → claim → quote → 5 CONCURRENT pay requests (one payout)
#      → a second payroll-api instance joins → the first is hard-killed during
#      confirmation polling → confirmed → repeat request is a duplicate
#   B. payroll-api hard-killed while the wallet builds the transaction
#      (interrupted submission) → restart → lease expiry → adopt → confirmed
#   C. wallet exits after recording the built tx, BEFORE broadcast → restart
#      → same bytes rebroadcast → confirmed
#   D. wallet exits after recording the intent, BEFORE building → restart →
#      intent failed → payout failed, claim released, no transaction exists
#   E. wallet exits after the wallet DB stored the signed tx but BEFORE the
#      intent recorded it → restart → recovered by memo → same tx → confirmed
#   F. wallet exits right AFTER the network accepted the tx, before recording
#      it → restart → rebroadcast of the same tx → confirmed
#   G. like E, and the intent store is LOST (moved aside) before restart →
#      payroll-side cancel finds the tx by memo and adopts it (no fence, no
#      second payment)
#   Then: carol's own wallet shows each paid payout exactly once with its memo;
#   the treasury has exactly one transaction per payout memo (none for D);
#   payout event histories; treasury accounting; server-rendered pages.
#
# Prerequisites (docs/PHASE3_PAYOUTS.md §7): wallets `treasury` (funded) and
# `carol`; `cargo build` in payroll-api and payout-wallet plus the fault binary
# (`cargo build --features fault-injection`, copied to
# hushush-payout-wallet-fault.exe); `npm run build` in web; local dev
# PostgreSQL and secrets in %LOCALAPPDATA%\hushush-dev. TESTNET ONLY.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
D="$LOCALAPPDATA/hushush-dev"
RUN_ID=${RUN_ID:-$(date -u +%Y%m%d%H%M%S)}
EVID="$ROOT/payout-wallet/evidence"
LOG="$EVID/live-$RUN_ID.log"
SVC="$EVID/live-$RUN_ID.services.log"
CLI="$ROOT/wallet-proof/target/debug/hushush-wallet-proof.exe"
WALLET_BIN="$ROOT/payout-wallet/target/debug/hushush-payout-wallet.exe"
FAULT_BIN="$ROOT/payout-wallet/target/debug/hushush-payout-wallet-fault.exe"
API_BIN="$ROOT/payroll-api/target/debug/hushush-payroll-api.exe"
INTENTS="$LOCALAPPDATA/hushush-phase0/wallets/treasury/payout-intents.sqlite"
LWD=${WALLET_LIGHTWALLETD_URL:-https://testnet.zec.rocks:443}
PAY_ZEC=0.0015
PAY_ZATS=150000
CONF=${PAYOUT_CONFIRMATIONS:-10}
SCENARIOS=${SCENARIOS:-ABCDEFG}
# RESUME=1: continue an interrupted run (same RUN_ID): reuse its database and
# state, restart the services, wait for RESUME_CLAIMS to confirm, verify
# against TREAS0 / CAROL_BEFORE taken from the original run's log.
RESUME=${RESUME:-0}
CLAIMS=()
FAILED_CLAIMS=()
mkdir -p "$EVID"
export PYTHONUTF8=1 NO_COLOR=1 RUST_LOG=${RUST_LOG:-info,zcash_client_backend=warn}

log() { echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$LOG"; }
die() { log "FAIL: $*"; exit 1; }
jget() { python -c "import sys,json;d=json.load(sys.stdin);print($1)"; }

set -a; . "$D/dev.env"; set +a
DB_URL="${DATABASE_URL%/*}/hushush_live_$RUN_ID"
# Not exported: only the wallet and payroll-api processes receive it (the web
# server must never see the wallet token).
WALLET_API_TOKEN=$(cat "$D/wallet-api-token.txt")
export PAYROLL_API_TOKEN; PAYROLL_API_TOKEN=$(python -c "import secrets;print(secrets.token_hex(32))")
export PAYROLL_API_URL=http://127.0.0.1:7880 APP_URL=http://localhost:3100
export STATE_FILE="$D/live-state-$RUN_ID.json"
LP() { (cd "$ROOT/web" && DATABASE_URL="$DB_URL" node scripts/live-payout.mjs "$@"); }
winpid() { cat "/proc/$1/winpid"; }
hard_kill() { taskkill //F //T //PID "$(winpid "$1")" >/dev/null 2>&1 || kill -9 "$1" 2>/dev/null || true; }
wait_http() { for _ in $(seq 1 "${2:-180}"); do curl -sf -o /dev/null "$1" && return 0; sleep 1; done; die "$1 never came up"; }
wstatus() { curl -s -H "Authorization: Bearer $WALLET_API_TOKEN" http://127.0.0.1:7979/v1/status; }

start_wallet() { # $1 = binary, rest = extra env
  local bin=$1; shift
  env "$@" WALLET_API_TOKEN="$WALLET_API_TOKEN" WALLET_NAME=treasury WALLET_LIGHTWALLETD_URL="$LWD" WALLET_SYNC_SECS=20 "$bin" >>"$SVC" 2>&1 &
  WALLET_PID=$!
  wait_http http://127.0.0.1:7979/healthz 600   # start-up retries lightwalletd for up to ~5 min
}
stop_wallet() { kill "$WALLET_PID" 2>/dev/null || hard_kill "$WALLET_PID"; wait "$WALLET_PID" 2>/dev/null || true; }
start_api() { # $1 = port (default 7880)
  local port=${1:-7880}
  PAYROLL_DATABASE_URL="$DB_URL" PAYROLL_API_ADDR=127.0.0.1:$port \
  WALLET_URL=http://127.0.0.1:7979 WALLET_API_TOKEN="$WALLET_API_TOKEN" PAYOUT_ENABLED_ORGS='*' PAYOUT_CONFIRMATIONS="$CONF" \
  PAYOUT_POLL_SECS=15 WALLET_SUBMIT_TIMEOUT_SECS=120 "$API_BIN" >>"$SVC" 2>&1 &
  LAST_API_PID=$!
  [ "$port" = 7880 ] && API_PID=$LAST_API_PID
  wait_http "http://127.0.0.1:$port/healthz"
}
port_pid() { netstat -ano | awk -v p=":$1" '$2 ~ p"$" && $4 == "LISTENING" {print $5; exit}'; }
cleanup() {
  for p in ${WEB_PID:-} ${API_PID:-} ${API2_PID:-} ${WALLET_PID:-} ${AWAKE_PID:-}; do hard_kill "$p"; done
  # `next start` runs as a grandchild; make sure nothing outlives the run.
  for port in 3100 7880 7882 7979; do
    w=$(port_pid $port); [ -n "$w" ] && taskkill //F //T //PID "$w" >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT
intent_of() { # payout id -> intent row (state, txid, broadcast attempts) from the CURRENT intent store
  python - "$INTENTS" "$1" <<'EOF'
import sqlite3, sys, json
c = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
r = c.execute("SELECT state, txid, broadcast_attempts, failure_reason FROM payment_intents WHERE payment_id = ?", (sys.argv[2],)).fetchone()
print(json.dumps({"state": r[0], "txid": r[1], "broadcast_attempts": r[2], "failure_reason": r[3]} if r else None))
EOF
}
memo_count() { # wallet name, payout id -> number of transactions carrying that payout's memo
  "$CLI" history "$1" | python -c "
import sys, json
h = json.load(sys.stdin); memo = 'hushush payout ' + sys.argv[1]
print(len({t['txid'] for t in h for o in t['outputs'] if o['memo'] == memo}))" "$2"
}

for port in 3100 7880 7882 7979; do
  [ -z "$(port_pid $port)" ] || { echo "port $port is already in use (stale run?); refusing to start" >&2; exit 1; }
done
# Keep the host awake for the duration of this run only (released on exit).
powershell -NoProfile -Command "Add-Type -Name P -Namespace W -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint f);'; [void][W.P]::SetThreadExecutionState(0x80000001); while (\$true) { Start-Sleep 60 }" >/dev/null 2>&1 &
AWAKE_PID=$!

log "=== hushush live payout run $RUN_ID (TESTNET, $LWD, $CONF confirmations, scenarios ${SCENARIOS:-resume})"
log "lightwalletd: $("$CLI" info)"
if [ "$RESUME" = 1 ]; then
  log "RESUMING run $RUN_ID (database, state and starting balances from the original run)"
  SCENARIOS=""
else
(cd "$ROOT/web" && node -e "
const pg=require('pg');const c=new pg.Client({connectionString:process.env.DATABASE_URL});
c.connect().then(()=>c.query('CREATE DATABASE hushush_live_$RUN_ID')).then(()=>c.end())")
log "database hushush_live_$RUN_ID created"
fi

start_wallet "$WALLET_BIN"
start_api
(cd "$ROOT/web" && DATABASE_URL="$DB_URL" npm run db:migrate >/dev/null && DATABASE_URL="$DB_URL" HUSHUSH_ALLOW_DEV_OUTBOX=1 npx next start -p 3100 >>"$D/live-web-$RUN_ID.log" 2>&1) &
WEB_PID=$!
wait_http http://localhost:3100/login
log "services up: wallet pid $WALLET_PID, payroll-api pid $API_PID, web pid $WEB_PID"

CAROL=$("$CLI" address carol)
if [ "$RESUME" = 1 ]; then
  read -r -a CLAIMS <<<"$RESUME_CLAIMS"
  for CL in "${CLAIMS[@]}"; do LP wait "$CL" confirmed 3000 | tee -a "$LOG"; done
else
CAROL_BEFORE=$("$CLI" sync carol 2>/dev/null | jget "d['total']")
TREAS0=$(wstatus | jget "d['total_zats']")
log "treasury total=$TREAS0 spendable=$(wstatus | jget "d['spendable_zats']"); carol total=$CAROL_BEFORE"
log "setup: $(LP setup)"
log "wages: $(LP wages)"
fi

wait_spendable() { # until a quote for $1 says sufficient
  for _ in $(seq 1 60); do
    q=$(LP quote "$1" "$CAROL")
    [ "$(echo "$q" | jget "d.get('sufficient')")" = "True" ] && { log "quote: $q"; return 0; }
    log "waiting for spendable treasury funds: $(echo "$q" | jget "d.get('reason')")"; sleep 20
  done
  die "treasury never became spendable"
}

# A wallet-side crash at fault point $2 while paying a new claim; prints the claim id.
fault_pay() { # $1 = letter, $2 = fault point
  local cl; cl=$(LP claim $PAY_ZEC); log "claim $1 $cl"
  wait_spendable "$cl"
  stop_wallet
  start_wallet "$FAULT_BIN" WALLET_FAULT="$2"; log "fault-injection wallet started (pid $WALLET_PID, WALLET_FAULT=$2)"
  local r; r=$(LP pay "$cl" "$CAROL"); log "pay $1: $r"
  [ "$(echo "$r" | jget "d.get('payout', {}).get('state')")" = requires_reconciliation ] || die "pay $1 did not reach requires_reconciliation"
  set +e; wait "$WALLET_PID"; local code=$?; set -e
  log "fault wallet exit code: $code (86 = injected crash at $2)"
  [ "$code" = 86 ] || die "fault point $2 did not fire"
  local p; p=$(LP payout-of "$cl"); log "payout $1 in DB: $p"
  FAULT_PAYOUT=$(echo "$p" | jget "d['id']")
  log "wallet intent $1 after crash: $(intent_of "$FAULT_PAYOUT")"
  FAULT_CLAIM=$cl
}

# ---------------------------------------------------------------- A
if [[ $SCENARIOS == *A* ]]; then
log "=== A: concurrent requests, second instance, crash during confirmation polling"
A=$(LP claim $PAY_ZEC); log "claim A $A"
wait_spendable "$A"
PAYS=()
for i in 1 2 3 4 5; do LP pay "$A" "$CAROL" >"$D/live-$RUN_ID-payA$i.json" 2>&1 & PAYS+=($!); done
for p in "${PAYS[@]}"; do wait "$p" || true; done
python - "$D/live-$RUN_ID-payA" <<'EOF' | tee -a "$LOG"
import json, sys
rs = [json.load(open(f"{sys.argv[1]}{i}.json")) for i in range(1, 6)]
ids = {r["payout"]["id"] for r in rs if "payout" in r}
created = [r for r in rs if r.get("status") == 201]
dups = [r for r in rs if r.get("status") == 200 and r.get("duplicate")]
print(f"5 concurrent pay requests: {len(created)} created, {len(dups)} duplicates, statuses {[r.get('status') for r in rs]}, distinct payout ids {len(ids)}")
assert len(created) == 1 and len(ids) == 1, rs
EOF
log "claim A while submitted: $(LP show "$A" | jget "(d['claim']['status'], d['claim']['payout']['state'])") (must be reserved = unpaid)"
start_api 7882; API2_PID=$LAST_API_PID; log "second payroll-api instance started (pid $API2_PID) — two workers now poll the same payouts"
sleep 30
hard_kill "$API_PID"; log "payroll-api #1 HARD-KILLED during confirmation polling (pid $API_PID)"
# Observe through the surviving instance while #1 is down.
PAYROLL_API_URL=http://127.0.0.1:7882 LP wait "$A" confirmed 2400 | tee -a "$LOG"
start_api; log "payroll-api #1 restarted (pid $API_PID)"
log "claim A after confirmation: $(LP show "$A" | jget "(d['claim']['status'], d['claim']['settlement_ref'], d['balance']['claimed_zec'])")"
R3=$(LP pay "$A" "$CAROL"); log "pay A after confirmation: $(echo "$R3" | jget "(d['status'], d['duplicate'], d['payout']['state'])")"
hard_kill "$API2_PID"; API2_PID=
CLAIMS+=("$A")
fi

# ---------------------------------------------------------------- B
if [[ $SCENARIOS == *B* ]]; then
log "=== B: payroll-api killed while the wallet builds the transaction"
B=$(LP claim $PAY_ZEC); log "claim B $B"
wait_spendable "$B"
LP pay "$B" "$CAROL" >"$D/live-$RUN_ID-payB.json" 2>&1 &
PAYB=$!
K=$(LP kill-when-reserved "$B" "$(winpid "$API_PID")" 300); log "crash: $K"
wait "$PAYB" || true; log "client saw: $(cat "$D/live-$RUN_ID-payB.json")"
log "payout B in DB while payroll-api is down: $(LP payout-of "$B")"
sleep 5; start_api; log "payroll-api restarted (pid $API_PID); the dead process's lease must expire first"
LP wait "$B" confirmed 3000 | tee -a "$LOG"
CLAIMS+=("$B")
fi

# ---------------------------------------------------------------- C
if [[ $SCENARIOS == *C* ]]; then
log "=== C: wallet exits after recording the built tx, before broadcasting"
fault_pay C crash_after_build
TXC=$(intent_of "$FAULT_PAYOUT" | jget "d['txid']")
log "chain view of C before restart: $("$CLI" observe "$TXC" 2>&1 | head -c 120 | tr '\n' ' ')"
start_wallet "$WALLET_BIN"; log "normal wallet restarted (pid $WALLET_PID)"
LP wait "$FAULT_CLAIM" confirmed 3000 | tee -a "$LOG"
CLAIMS+=("$FAULT_CLAIM")
fi

# ---------------------------------------------------------------- D
if [[ $SCENARIOS == *D* ]]; then
log "=== D: wallet exits after recording the intent, before building"
fault_pay D crash_before_build
start_wallet "$WALLET_BIN"; log "normal wallet restarted (pid $WALLET_PID)"
log "wallet intent D after recovery: $(intent_of "$FAULT_PAYOUT")"
LP wait "$FAULT_CLAIM" failed 900 | tee -a "$LOG"
log "claim D: $(LP show "$FAULT_CLAIM" | jget "(d['claim']['status'], d['claim']['payout']['detail'])") (must be released: nothing was sent)"
[ "$(memo_count treasury "$FAULT_PAYOUT")" = 0 ] || die "a transaction exists for failed payout D"
log "treasury holds 0 transactions for payout D"
FAILED_CLAIMS+=("$FAULT_CLAIM")
fi

# ---------------------------------------------------------------- E
if [[ $SCENARIOS == *E* ]]; then
log "=== E: wallet exits after storing the signed tx, before recording it in the intent store"
fault_pay E crash_after_wallet_store
start_wallet "$WALLET_BIN"; log "normal wallet restarted (pid $WALLET_PID); start-up recovery searches the wallet DB by memo"
log "wallet intent E after recovery: $(intent_of "$FAULT_PAYOUT")"
LP wait "$FAULT_CLAIM" confirmed 3000 | tee -a "$LOG"
CLAIMS+=("$FAULT_CLAIM")
fi

# ---------------------------------------------------------------- F
if [[ $SCENARIOS == *F* ]]; then
log "=== F: wallet exits right after the network accepted the tx, before recording the broadcast"
fault_pay F crash_after_broadcast
TXF=$(intent_of "$FAULT_PAYOUT" | jget "d['txid']")
log "chain view of F before restart: $("$CLI" observe "$TXF" 2>&1 | head -c 160 | tr '\n' ' ')"
start_wallet "$WALLET_BIN"; log "normal wallet restarted (pid $WALLET_PID)"
LP wait "$FAULT_CLAIM" confirmed 3000 | tee -a "$LOG"
log "wallet intent F after recovery: $(intent_of "$FAULT_PAYOUT")"
CLAIMS+=("$FAULT_CLAIM")
fi

# ---------------------------------------------------------------- G
if [[ $SCENARIOS == *G* ]]; then
log "=== G: like E, and the wallet's intent store is LOST before restart"
fault_pay G crash_after_wallet_store
LOST="$INTENTS.lost-$RUN_ID"
for ext in "" -wal -shm; do [ -f "$INTENTS$ext" ] && mv "$INTENTS$ext" "$LOST$ext"; done
log "intent store moved aside to $(basename "$LOST") (simulated loss); the wallet starts with an empty one"
start_wallet "$WALLET_BIN"; log "normal wallet restarted (pid $WALLET_PID)"
LP wait "$FAULT_CLAIM" confirmed 3000 | tee -a "$LOG"
log "wallet intent G (adopted): $(intent_of "$FAULT_PAYOUT")"
CLAIMS+=("$FAULT_CLAIM")
fi

# ---------------------------------------------------------------- verification
log "=== verification"
"$CLI" sync carol >/dev/null 2>&1 || "$CLI" sync carol >/dev/null
TOTAL_FEES=0
PAID_TXS=()
for CL in "${CLAIMS[@]}"; do
  P=$(LP show "$CL")
  PID_=$(echo "$P" | jget "d['claim']['payout']['id']"); TX=$(echo "$P" | jget "d['claim']['payout']['txid']")
  FEE=$(echo "$P" | jget "d['claim']['payout']['fee_zats']"); TOTAL_FEES=$((TOTAL_FEES + FEE))
  log "payout $PID_: claim=$(echo "$P" | jget "d['claim']['status']") txid=$TX fee=$FEE events=$(LP events "$CL")"
  "$CLI" history carol | python -c "
import sys, json
h = json.load(sys.stdin); memo = 'hushush payout ' + sys.argv[1]
hits = [(t['txid'], o['value_zats']) for t in h for o in t['outputs'] if not o['is_sent'] and o['memo'] == memo]
assert hits == [(sys.argv[2], int(sys.argv[3]))], hits
print('carol received exactly once:', hits[0][1], 'zats, memo', repr(memo))
" "$PID_" "$TX" "$PAY_ZATS" | tee -a "$LOG"
  N_TX=$(memo_count treasury "$PID_"); log "treasury transactions carrying this payout's memo: $N_TX"
  [ "$N_TX" = 1 ] || die "payout $PID_ has $N_TX transactions"
  PAID_TXS+=("$TX")
done
for CL in "${FAILED_CLAIMS[@]}"; do
  PID_=$(LP show "$CL" | jget "d['claim']['payout']['id']")
  [ "$(memo_count carol "$PID_")" = 0 ] || die "carol received failed payout $PID_"
  log "failed payout $PID_: carol received nothing, as required"
done
CAROL_AFTER=$("$CLI" balance carol | jget "d['total']")
N=${#CLAIMS[@]}
log "carol total $CAROL_BEFORE → $CAROL_AFTER (delta $((CAROL_AFTER - CAROL_BEFORE)), expected $((N * PAY_ZATS)))"
[ $((CAROL_AFTER - CAROL_BEFORE)) -eq $((N * PAY_ZATS)) ] || die "carol delta wrong"
sleep 25
TREAS1=$(wstatus | jget "d['total_zats']")
log "treasury total $TREAS0 → $TREAS1 (delta $((TREAS0 - TREAS1)), expected $((N * PAY_ZATS + TOTAL_FEES)) = $N payouts + fees)"
[ $((TREAS0 - TREAS1)) -eq $((N * PAY_ZATS + TOTAL_FEES)) ] || die "treasury accounting wrong"
PAGES=$(LP pages "${CLAIMS[0]}"); log "pages: $PAGES"
echo "$PAGES" | python -c "
import json,sys; p=json.load(sys.stdin)
assert p['contributor_stream_page']==200 and p['owner_org_page']==200, p
assert 'Paid \u00b7 payout confirmed' in p['claim_labels_on_page'], p  # ASCII escape: console code pages mangle the middle dot
assert p['owner_sees_payout_wallet'], p" || die "server-rendered pages wrong"
log "paid txids: ${PAID_TXS[*]}"
log "PASS (${SCENARIOS:-resumed}): live testnet payouts submitted, confirmed, reconciled after crashes, verified; no duplicate payments"
