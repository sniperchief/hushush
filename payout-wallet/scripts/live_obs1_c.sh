#!/usr/bin/env bash
# hushush LIVE TESTNET validation of the OBS-1 fix: scenario C ONLY
# (docs/OBS1_WALLET_RECOVERY_FIX.md, docs/VESTING_PHASE4_FAULT_INJECTION.md).
#
# Scenario C: the wallet dies right AFTER lightwalletd accepted the
# transaction (`crash_after_broadcast`), so its intent stays `built` with 0
# recorded attempts. We wait until the transaction is MINED (the OBS-1
# condition), then start the normal wallet. Expected with the fix: start-up
# recovery looks the txid up, finds it mined, records the intent as broadcast
# WITHOUT a send (attempts stay 0) and never re-sends it; payroll settles only
# at PAYOUT_CONFIRMATIONS with chain verification.
#
# No faucet: the run works on a COPY of the Phase 4 run's database
# (CREATE DATABASE … TEMPLATE hushush_live_v4run2) and reuses that run's
# company, whose payout account still holds funds. The employer creates one
# new agreement (the old run's address key is gone, so the old agreements'
# sealed addresses can't be opened). The template database is not modified.
#
# Broadcasts: exactly one transaction (the withdrawal). If (and only if) the
# lookup reports the transaction definitively MISSING, the wallet re-sends the
# same stored bytes (same txid); this script then stops (unexpected result).
#
# TESTNET ONLY. Requires LIVE_AUTHORIZED=testnet. Ports 3300/7980/7979 free.
set -euo pipefail
[ "${LIVE_AUTHORIZED:-}" = testnet ] || { echo "refusing: set LIVE_AUTHORIZED=testnet to broadcast on Zcash TESTNET" >&2; exit 2; }

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
D="$LOCALAPPDATA/hushush-dev"
RUN_ID=${RUN_ID:-obs1c-$(date -u +%Y%m%d%H%M%S)}
SOURCE_RUN=${SOURCE_RUN:-v4run2}
EVID="$ROOT/payout-wallet/evidence"
LOG="$EVID/live-$RUN_ID.log"
# One log file per process: concurrent appends to one file lose lines on Windows.
WLOG="$EVID/live-$RUN_ID.wallet.log"
ALOG="$EVID/live-$RUN_ID.payroll-api.log"
WEBLOG="$EVID/live-$RUN_ID.web.log"
CLI="$ROOT/wallet-proof/target/debug/hushush-wallet-proof.exe"
WALLET_BIN="$ROOT/payout-wallet/target/debug/hushush-payout-wallet.exe"
FAULT_BIN="$ROOT/payout-wallet/target/debug/hushush-payout-wallet-fault.exe"
API_BIN="$ROOT/payroll-api/target/debug/hushush-payroll-api.exe"
INTENTS="$LOCALAPPDATA/hushush-phase0/wallets/treasury/payout-intents.sqlite"
LWD=${WALLET_LIGHTWALLETD_URL:-https://testnet.zec.rocks:443}
AMOUNT_ZEC=${AMOUNT_ZEC:-0.0002}
CONF=${PAYOUT_CONFIRMATIONS:-10}
mkdir -p "$EVID"
export PYTHONUTF8=1 NO_COLOR=1 RUST_LOG=${RUST_LOG:-info,zcash_client_backend=warn}

log() { echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" | tee -a "$LOG"; }
die() { log "STOP: $*"; log "evidence preserved: $LOG, $WLOG, $ALOG, database ${DB_NAME:-?}"; exit 1; }
jget() { python -c "import sys,json;d=json.load(sys.stdin);print($1)"; }

set -a; . "$D/dev.env"; set +a
SRC_DB="hushush_live_${SOURCE_RUN//-/_}"
DB_NAME="hushush_live_${RUN_ID//-/_}"
DB_URL="${DATABASE_URL%/*}/$DB_NAME"
WALLET_API_TOKEN=$(cat "$D/wallet-api-token.txt")
PAYROLL_API_TOKEN=$(python -c "import secrets;print(secrets.token_hex(32))")
PAYOUT_ADDRESS_KEY=$(python -c "import secrets;print(secrets.token_hex(32))")
export APP_URL=http://localhost:3300 STATE_FILE="$D/live-state-$RUN_ID.json" PAYROLL_API_URL=http://127.0.0.1:7980 PAYROLL_API_TOKEN
LV() { (cd "$ROOT/web" && node scripts/live-vesting.mjs "$@"); }
LA() { local a=$1; shift; (cd "$ROOT/web" && AGREEMENT="$a" node scripts/live-vesting.mjs "$@"); }
winpid() { cat "/proc/$1/winpid"; }
hard_kill() { taskkill //F //T //PID "$(winpid "$1")" >/dev/null 2>&1 || kill -9 "$1" 2>/dev/null || true; }
wait_http() { for _ in $(seq 1 "${2:-180}"); do curl -sf -o /dev/null "$1" && return 0; sleep 1; done; die "$1 never came up"; }
wcurl() { curl -s -H "Authorization: Bearer $WALLET_API_TOKEN" "$@"; }
wstatus() { wcurl http://127.0.0.1:7979/v1/status; }
port_pid() { netstat -ano | awk -v p=":$1" '$2 ~ p"$" && $4 == "LISTENING" {print $5; exit}'; }
cleanup() {
  for p in ${WEB_PID:-} ${API_PID:-} ${WALLET_PID:-} ${AWAKE_PID:-}; do hard_kill "$p"; done
  for port in 3300 7980 7979; do
    w=$(port_pid $port); [ -n "$w" ] && taskkill //F //T //PID "$w" >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT
memo_count() { "$CLI" history "$1" | python -c "
import sys, json
h = json.load(sys.stdin); memo = 'hushush payout ' + sys.argv[1]
print(len({t['txid'] for t in h for o in t['outputs'] if o['memo'] == memo}))" "$2"; }
intent_of() {
  python - "$INTENTS" "$1" <<'EOF'
import sqlite3, sys, json
c = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
r = c.execute("SELECT state, txid, broadcast_attempts, last_broadcast_error, fee_zats, raw_tx IS NOT NULL, updated_at FROM payment_intents WHERE payment_id = ?", (sys.argv[2],)).fetchone()
print(json.dumps({"state": r[0], "txid": r[1], "broadcast_attempts": r[2], "last_broadcast_error": r[3], "fee_zats": r[4], "raw_persisted": bool(r[5]), "updated_at": r[6]} if r else None))
EOF
}
dbq() { (cd "$ROOT/web" && DATABASE_URL="${2:-$DB_URL}" node -e "
const pg=require('pg');const c=new pg.Client({connectionString:process.env.DATABASE_URL});
c.connect().then(()=>c.query(process.argv[1])).then(r=>{console.log(JSON.stringify(r.rows));return c.end()})" "$1"); }
pstate() { LA C2 show | python -c "import sys,json; d=json.load(sys.stdin); p=d['payouts'][0]; c=d['claims'][0]; print(json.dumps({'payout': p['state'], 'confirmations': p['confirmations'], 'txid': p['txid'], 'claim': c['status'], 'reserved': d['balance']['reserved_zats'], 'paid': d['balance']['paid_zats']}))"; }
start_wallet() { local bin=$1; shift
  env "$@" WALLET_API_TOKEN="$WALLET_API_TOKEN" WALLET_NAME=treasury WALLET_LIGHTWALLETD_URL="$LWD" WALLET_SYNC_SECS=20 "$bin" >>"$WLOG" 2>&1 &
  WALLET_PID=$!; wait_http http://127.0.0.1:7979/healthz 600; }
stop_wallet() { kill "$WALLET_PID" 2>/dev/null || hard_kill "$WALLET_PID"; wait "$WALLET_PID" 2>/dev/null || true; sleep 2; [ -z "$(port_pid 7979)" ] || die "wallet still listening"; }
start_api() {
  PAYROLL_DATABASE_URL="$DB_URL" PAYROLL_API_ADDR=127.0.0.1:7980 PAYROLL_API_TOKEN="$PAYROLL_API_TOKEN" \
  WALLET_URL=http://127.0.0.1:7979 WALLET_API_TOKEN="$WALLET_API_TOKEN" PAYOUT_ENABLED_ORGS='*' PAYOUT_CONFIRMATIONS="$CONF" \
  PAYOUT_ADDRESS_KEY="$PAYOUT_ADDRESS_KEY" PAYOUT_POLL_SECS=15 WALLET_SUBMIT_TIMEOUT_SECS=180 "$API_BIN" >>"$ALOG" 2>&1 &
  API_PID=$!; wait_http http://127.0.0.1:7980/healthz; }

# ------------------------------------------------------------------ preflight
for port in 3300 7980 7979; do [ -z "$(port_pid $port)" ] || { echo "port $port is already in use; refusing to start" >&2; exit 1; }; done
for b in "$CLI" "$WALLET_BIN" "$FAULT_BIN" "$API_BIN"; do [ -f "$b" ] || { echo "missing $b" >&2; exit 1; }; done
[ -f "$D/live-state-$SOURCE_RUN.json" ] || { echo "missing $SOURCE_RUN state" >&2; exit 1; }
powershell -NoProfile -Command "Add-Type -Name P -Namespace W -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint f);'; [void][W.P]::SetThreadExecutionState(0x80000001); while (\$true) { Start-Sleep 60 }" >/dev/null 2>&1 &
AWAKE_PID=$!

log "=== hushush OBS-1 live validation $RUN_ID: scenario C only (TESTNET, $LWD, $CONF confirmations, $AMOUNT_ZEC ZEC, company from $SOURCE_RUN)"
log "binaries: wallet $(sha256sum "$WALLET_BIN" | cut -c1-16) fault $(sha256sum "$FAULT_BIN" | cut -c1-16) payroll-api $(sha256sum "$API_BIN" | cut -c1-16)"
INFO=$("$CLI" info); log "lightwalletd: $INFO"
case "$INFO" in *" chain=test "*"branch_id=77190ad9"*) log "network check: chain=test, NU7 testnet branch 77190ad9" ;; *) die "lightwalletd is not the NU7 testnet" ;; esac
(cd "$ROOT/web" && node -e "
const pg=require('pg');const c=new pg.Client({connectionString:process.env.DATABASE_URL});
c.connect().then(()=>c.query('CREATE DATABASE $DB_NAME TEMPLATE $SRC_DB')).then(()=>c.end())")
log "database $DB_NAME created as a copy of $SRC_DB (template untouched)"
python - "$D/live-state-$SOURCE_RUN.json" "$STATE_FILE" <<'EOF'
import json, sys
s = json.load(open(sys.argv[1])); s["agreements"] = {}; json.dump(s, open(sys.argv[2], "w"))
EOF
ORG=$(jget "d['organizationId']" < "$STATE_FILE")

start_wallet "$WALLET_BIN"
start_api
(cd "$ROOT/web" && DATABASE_URL="$DB_URL" npm run db:migrate >/dev/null && DATABASE_URL="$DB_URL" PAYROLL_API_URL=http://127.0.0.1:7980 PAYROLL_API_TOKEN="$PAYROLL_API_TOKEN" npx next start -p 3300 >>"$WEBLOG" 2>&1) &
WEB_PID=$!
wait_http http://localhost:3300/login
log "services up: wallet pid $WALLET_PID, payroll-api pid $API_PID, web pid $WEB_PID"
log "payroll schema: $(dbq "select max(version::int) as v from payroll_schema_migrations")"

CAROL=$("$CLI" address carol)
CAROL_BEFORE=$("$CLI" sync carol 2>/dev/null | jget "d['total']")
TREAS0=$(wstatus | jget "d['total_zats']")
COMPANY0=$(wcurl "http://127.0.0.1:7979/v1/accounts/$ORG" | jget "d['total_zats']")
UNF0=$(wstatus | jget "d['unfinished_payments']")
log "treasury total=$TREAS0; company account $ORG total=$COMPANY0; carol total=$CAROL_BEFORE; wallet unfinished payments=$UNF0"
# The fault wallet's start-up resend pass must not meet any other unsent transaction.
[ "$UNF0" = 0 ] || die "the wallet has $UNF0 unfinished payments; refusing to start a crash_after_broadcast wallet"
AMOUNT=$(python -c "from decimal import Decimal; print(int(Decimal('$AMOUNT_ZEC')*100000000))")

log "=== setup: one new agreement on the existing company (no faucet)"
log "agreement C2: $(LV create C2 1 90)"
R=$(LA C2 address "$CAROL"); [ "$(echo "$R" | jget "d['address_locked']")" = True ] || die "address not locked: $R"
log "carol's address locked on C2"
log "available: $(LA C2 wait-available "$AMOUNT" 600 | jget "d['balance']['available_zats']")"
Q=$(LA C2 quote "$AMOUNT_ZEC"); log "quote: $Q"
[ "$(echo "$Q" | jget "d.get('sufficient')")" = True ] || die "company account cannot pay (no faucet in this plan): $Q"

log "=== C: wallet dies right AFTER the network accepted the transaction"
stop_wallet
start_wallet "$FAULT_BIN" WALLET_FAULT=crash_after_broadcast; log "fault-injection wallet started (pid $WALLET_PID, WALLET_FAULT=crash_after_broadcast)"
R=$(LA C2 withdraw "$AMOUNT_ZEC"); log "withdraw: $R"
[ "$(echo "$R" | jget "d['status']")" = 201 ] || die "withdrawal not created"
[ "$(echo "$R" | jget "d['payout']['state']")" = requires_reconciliation ] || die "withdrawal did not reach requires_reconciliation"
set +e; wait "$WALLET_PID"; CODE=$?; set -e
log "fault wallet exit code: $CODE (86 = injected crash at crash_after_broadcast)"
[ "$CODE" = 86 ] || die "fault point did not fire"
P=$(echo "$R" | jget "d['payout']['id']")
BEFORE=$(intent_of "$P"); log "intent BEFORE recovery: $BEFORE"
TX=$(echo "$BEFORE" | jget "d['txid']")
[ "$(echo "$BEFORE" | jget "d['state']")" = built ] && [ "$(echo "$BEFORE" | jget "d['broadcast_attempts']")" = 0 ] || die "unexpected intent after the crash"
log "payroll while the wallet is down: $(pstate)"

log "=== waiting (wallet down) until the transaction is MINED: the OBS-1 condition"
MINED=""
for _ in $(seq 1 60); do
  O=$("$CLI" observe "$TX" 2>&1 || true)
  MINED=$(echo "$O" | python -c "import sys,json
try: print(json.load(sys.stdin).get('mined_height') or '')
except Exception: print('')")
  [ -n "$MINED" ] && break
  sleep 15
done
log "chain view before restart: $(echo "$O" | head -c 240 | tr '\n' ' ')"
[ -n "$MINED" ] || die "transaction not mined within 15 minutes; not restarting the wallet (no blind rebroadcast)"
log "transaction $TX mined at $MINED while the wallet was down"
S=$(pstate); log "payroll while the wallet is down: $S"
echo "$S" | python -c "import sys,json; s=json.load(sys.stdin); assert s['payout']=='requires_reconciliation' and s['claim']=='reserved' and s['paid']==0, s" || die "payroll resolved while the wallet was down"

log "=== restart with the FIXED normal wallet"
WL0=$(wc -l < "$WLOG")
start_wallet "$WALLET_BIN"; log "normal wallet started (pid $WALLET_PID); start-up recovery ran before it listened"
AFTER=$(intent_of "$P"); log "intent AFTER start-up recovery: $AFTER"
PLINES=$(tail -n +"$((WL0 + 1))" "$WLOG" | sed -E 's/\x1b\[[0-9;]*m//g' | grep -F "$P" | cut -c1-400 || true)
log "wallet log lines for this payment since restart:"; echo "$PLINES" | tee -a "$LOG"
python - "$BEFORE" "$AFTER" <<'EOF' | tee -a "$LOG" || die "intent did not move as expected (see wallet log)"
import json, sys
b, a = json.loads(sys.argv[1]), json.loads(sys.argv[2])
assert a["state"] == "broadcast", a
assert a["broadcast_attempts"] == b["broadcast_attempts"] == 0, (b, a)
assert a["txid"] == b["txid"] and a["fee_zats"] == b["fee_zats"] and a["raw_persisted"], (b, a)
print(f"intent: {b['state']} → {a['state']}, attempts {b['broadcast_attempts']} → {a['broadcast_attempts']}, same txid {a['txid'][:16]}…")
EOF
grep -q "recorded as broadcast without resending" <<<"$PLINES" || die "the observation log line is missing"
if grep -qE "broadcast accepted|broadcast failed|chain lookup failed|lightwalletd returned transaction" <<<"$PLINES"; then die "a send, a failed lookup or a mismatched transaction was logged for this payment"; fi
log "no send attempted for $P after restart"

log "=== three background syncs later (WALLET_SYNC_SECS=20): still no attempt"
sleep 70
LATER=$(intent_of "$P"); log "intent 70 s later: $LATER"
[ "$(echo "$LATER" | jget "d['broadcast_attempts']")" = 0 ] && [ "$(echo "$LATER" | jget "d['state']")" = broadcast ] || die "attempts changed after the observation"
log "wallet unfinished payments now: $(wstatus | jget "d['unfinished_payments']") (before the run: $UNF0)"

log "=== payroll: settles only at $CONF confirmations with chain verification"
LA C2 wait "$P" confirmed 3600 | tee -a "$LOG"
FINAL=$(LA C2 show); log "agreement C2 final: $FINAL"
log "payout history: $(dbq "select to_state from payout_events where payout_id = '$P' order by id" | python -c "import sys,json; print(' > '.join(r['to_state'] for r in json.load(sys.stdin)))")"
log "claim: $(dbq "select status, settlement_ref = (select txid from payouts where id = '$P') as ref_is_txid from vesting_claims where id = (select vesting_claim_id from payouts where id = '$P')")"
LAST=$(intent_of "$P"); log "intent at the end: $LAST"
[ "$(echo "$LAST" | jget "d['broadcast_attempts']")" = 0 ] || die "attempts changed before confirmation"

log "=== verification"
"$CLI" sync carol >/dev/null 2>&1 || "$CLI" sync carol >/dev/null
"$CLI" history carol | python -c "
import sys, json
h = json.load(sys.stdin); memo = 'hushush payout ' + sys.argv[1]
hits = [(t['txid'], o['value_zats']) for t in h for o in t['outputs'] if not o['is_sent'] and o['memo'] == memo]
assert hits == [(sys.argv[2], int(sys.argv[3]))], hits
print('carol received exactly once:', hits[0][1], 'zats in', sys.argv[2])" "$P" "$TX" "$AMOUNT" | tee -a "$LOG" || die "carol receipt check failed"
N=$(memo_count treasury "$P"); log "treasury-wallet transactions carrying the payout memo: $N"; [ "$N" = 1 ] || die "$N transactions"
CAROL_AFTER=$("$CLI" balance carol | jget "d['total']")
log "carol $CAROL_BEFORE → $CAROL_AFTER (delta $((CAROL_AFTER - CAROL_BEFORE)), expected $AMOUNT)"; [ $((CAROL_AFTER - CAROL_BEFORE)) -eq "$AMOUNT" ] || die "carol delta"
sleep 25
FEE=$(echo "$FINAL" | jget "d['payouts'][0]['fee_zats']")
COMPANY1=$(wcurl "http://127.0.0.1:7979/v1/accounts/$ORG" | jget "d['total_zats']"); TREAS1=$(wstatus | jget "d['total_zats']")
log "company account $COMPANY0 → $COMPANY1 (delta $((COMPANY0 - COMPANY1)), expected $((AMOUNT + FEE)))"; [ $((COMPANY0 - COMPANY1)) -eq $((AMOUNT + FEE)) ] || die "company delta"
log "treasury $TREAS0 → $TREAS1 (expected unchanged: no faucet)"; [ "$TREAS0" = "$TREAS1" ] || die "treasury changed"
log "payout $P txid $TX"
log "PASS: OBS-1 scenario C — mined transaction recorded as broadcast without a send (attempts 0), never re-sent, settled once at $CONF confirmations"
