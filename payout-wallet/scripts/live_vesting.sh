#!/usr/bin/env bash
# hushush LIVE TESTNET run of a vesting withdrawal (Vesting Phase 3).
#
# Real everything except the passkey authenticator (a SOFTWARE one): the
# Next.js app (`next start`), payroll-api, PostgreSQL (a NEW database per
# run), the payout wallet service on the `treasury` wallet, lightwalletd on
# the NU7 testnet. The employee's payout address is `carol`'s (CLI wallet).
#
#   1. employer signs up, names the company, asks the testnet faucet to fund
#      the company's payout account; the employee joins through a worker link
#      (membership); the employer creates a linear vesting agreement
#      (VEST_TOTAL over 1 day, no cliff, starting VEST_START_IN s from now)
#      and activates it before its start
#   2. invalid address refused by the real wallet; carol's address locked; a
#      change refused; a withdrawal before the start refused
#   3. wait until AMOUNT has vested and the company account can pay; quote
#   4. ONE withdrawal (one broadcast). A retry with the same key, and a new
#      key while it is in flight, must return the same payout (no new tx)
#   5. payroll-api (this run's own instance) is stopped and started again
#      while the payout confirms: the new process continues it
#   6. wait for PAYOUT_CONFIRMATIONS; verify claim settled once, accounting,
#      carol received exactly once, exactly one wallet tx carries the memo,
#      company account delta, pages
#
# TESTNET ONLY. Requires LIVE_AUTHORIZED=testnet (explicit operator consent).
# Ports 3300 (web), 7980 (payroll-api) and 7979 (wallet) must be free.
set -euo pipefail
[ "${LIVE_AUTHORIZED:-}" = testnet ] || { echo "refusing: set LIVE_AUTHORIZED=testnet to broadcast on Zcash TESTNET" >&2; exit 2; }

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
D="$LOCALAPPDATA/hushush-dev"
RUN_ID=${RUN_ID:-v3-$(date -u +%Y%m%d%H%M%S)}
EVID="$ROOT/payout-wallet/evidence"
LOG="$EVID/live-$RUN_ID.log"
SVC="$EVID/live-$RUN_ID.services.log"
CLI="$ROOT/wallet-proof/target/debug/hushush-wallet-proof.exe"
WALLET_BIN="$ROOT/payout-wallet/target/debug/hushush-payout-wallet.exe"
API_BIN=${API_BIN:-$ROOT/payroll-api/target/debug/hushush-payroll-api.exe}
LWD=${WALLET_LIGHTWALLETD_URL:-https://testnet.zec.rocks:443}
VEST_TOTAL=${VEST_TOTAL:-0.05}
VEST_START_IN=${VEST_START_IN:-90}
AMOUNT_ZEC=${AMOUNT_ZEC:-0.0003}
CONF=${PAYOUT_CONFIRMATIONS:-10}
mkdir -p "$EVID"
export PYTHONUTF8=1 NO_COLOR=1 RUST_LOG=${RUST_LOG:-info,zcash_client_backend=warn}

log() { echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" | tee -a "$LOG"; }
die() { log "FAIL: $*"; exit 1; }
jget() { python -c "import sys,json;d=json.load(sys.stdin);print($1)"; }

set -a; . "$D/dev.env"; set +a
DB_NAME="hushush_live_${RUN_ID//-/_}"
DB_URL="${DATABASE_URL%/*}/$DB_NAME"
WALLET_API_TOKEN=$(cat "$D/wallet-api-token.txt")
PAYROLL_API_TOKEN=$(python -c "import secrets;print(secrets.token_hex(32))")
PAYOUT_ADDRESS_KEY=$(python -c "import secrets;print(secrets.token_hex(32))")
export APP_URL=http://localhost:3300 STATE_FILE="$D/live-state-$RUN_ID.json" PAYROLL_API_URL=http://127.0.0.1:7980 PAYROLL_API_TOKEN
LV() { (cd "$ROOT/web" && node scripts/live-vesting.mjs "$@"); }
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
start_api() {
  PAYROLL_DATABASE_URL="$DB_URL" PAYROLL_API_ADDR=127.0.0.1:7980 PAYROLL_API_TOKEN="$PAYROLL_API_TOKEN" \
  WALLET_URL=http://127.0.0.1:7979 WALLET_API_TOKEN="$WALLET_API_TOKEN" PAYOUT_ENABLED_ORGS='*' PAYOUT_CONFIRMATIONS="$CONF" \
  PAYOUT_ADDRESS_KEY="$PAYOUT_ADDRESS_KEY" PAYOUT_POLL_SECS=15 WALLET_SUBMIT_TIMEOUT_SECS=180 "$API_BIN" >>"$SVC" 2>&1 &
  API_PID=$!
  wait_http http://127.0.0.1:7980/healthz
}

for port in 3300 7980 7979; do
  [ -z "$(port_pid $port)" ] || { echo "port $port is already in use; refusing to start" >&2; exit 1; }
done
powershell -NoProfile -Command "Add-Type -Name P -Namespace W -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint f);'; [void][W.P]::SetThreadExecutionState(0x80000001); while (\$true) { Start-Sleep 60 }" >/dev/null 2>&1 &
AWAKE_PID=$!

log "=== hushush Vesting Phase 3 live run $RUN_ID (TESTNET, $LWD, $CONF confirmations, agreement $VEST_TOTAL ZEC over 1 day, withdraw $AMOUNT_ZEC ZEC)"
INFO=$("$CLI" info); log "lightwalletd: $INFO"
case "$INFO" in *" chain=test "*"branch_id=77190ad9"*) log "network check: chain=test, NU7 testnet branch 77190ad9" ;; *) die "lightwalletd is not the NU7 testnet" ;; esac
(cd "$ROOT/web" && node -e "
const pg=require('pg');const c=new pg.Client({connectionString:process.env.DATABASE_URL});
c.connect().then(()=>c.query('CREATE DATABASE $DB_NAME')).then(()=>c.end())")
log "database $DB_NAME created (fresh; payroll-api migrates it on start)"

WALLET_API_TOKEN="$WALLET_API_TOKEN" WALLET_NAME=treasury WALLET_LIGHTWALLETD_URL="$LWD" WALLET_SYNC_SECS=20 "$WALLET_BIN" >>"$SVC" 2>&1 &
WALLET_PID=$!
wait_http http://127.0.0.1:7979/healthz 600
start_api
(cd "$ROOT/web" && DATABASE_URL="$DB_URL" npm run db:migrate >/dev/null && DATABASE_URL="$DB_URL" PAYROLL_API_URL=http://127.0.0.1:7980 PAYROLL_API_TOKEN="$PAYROLL_API_TOKEN" npx next start -p 3300 >>"$SVC" 2>&1) &
WEB_PID=$!
wait_http http://localhost:3300/login
log "services up: wallet pid $WALLET_PID, payroll-api pid $API_PID, web pid $WEB_PID"
log "payroll schema: $(cd "$ROOT/web" && DATABASE_URL="$DB_URL" node -e "
const pg=require('pg');const c=new pg.Client({connectionString:process.env.DATABASE_URL});
c.connect().then(()=>c.query(\"select to_regclass('vesting_agreements') is not null as vesting, (select count(*) from information_schema.columns where table_name='payouts' and column_name in ('compensation_type','vesting_claim_id','vesting_agreement_id'))::int as payout_cols\")).then(r=>{console.log(JSON.stringify(r.rows[0]));return c.end()})")"

CAROL=$("$CLI" address carol)
ALICE=$("$CLI" address alice)
CAROL_BEFORE=$("$CLI" sync carol 2>/dev/null | jget "d['total']")
TREAS0=$(wstatus | jget "d['total_zats']")
log "treasury total=$TREAS0 spendable=$(wstatus | jget "d['spendable_zats']"); carol total=$CAROL_BEFORE"

log "=== 1: employer, faucet, employee membership, vesting agreement"
SETUP=$(LV setup "$VEST_TOTAL" "$VEST_START_IN"); log "setup: $SETUP"
FAUCET_ID=$(echo "$SETUP" | jget "d['faucet']['payment_id']"); FAUCET_AMT=$(echo "$SETUP" | jget "d['faucet']['amount_zats']")
ORG=$(echo "$SETUP" | jget "d['organizationId']")
[ "$(echo "$SETUP" | jget "d['faucet']['state']")" = broadcast ] || die "faucet top-up was not broadcast"
[ "$(echo "$SETUP" | jget "d['status']")" = active ] || die "agreement not active"

log "=== 2: address and pre-start checks"
R=$(LV address utest1thisisnotarealunifiedaddressatall); log "invalid address: $R"
[ "$(echo "$R" | jget "d['status']")" = 422 ] || die "the real wallet did not refuse an invalid address"
R=$(LV address "$CAROL"); log "carol's address: status $(echo "$R" | jget "d['status']") locked $(echo "$R" | jget "d['address_locked']")"
[ "$(echo "$R" | jget "d['address_locked']")" = True ] || die "address not locked"
R=$(LV address "$ALICE"); log "change to another address: $(echo "$R" | jget "{k:v for k,v in d.items() if k!='address_locked'}")"
[ "$(echo "$R" | jget "d['status']")" = 409 ] || die "the address lock did not hold"
R=$(LV withdraw "$AMOUNT_ZEC"); log "withdraw before the start: $R"
python -c "import sys,json; r=json.loads(sys.argv[1]); assert r['status'] >= 400 and r['payout'] is None, r" "$R" || die "withdrawal before the start was not refused"

AMOUNT=$(python -c "from decimal import Decimal; print(int(Decimal('$AMOUNT_ZEC')*100000000))")
log "=== 3: wait for $AMOUNT zats vested and the company account funded"
log "company wallet: $(LV wait-funded $((AMOUNT + 40000)) 1500)"
log "available: $(LV wait-available "$AMOUNT" 1800)"
Q=$(LV quote "$AMOUNT_ZEC"); log "quote: $Q"
[ "$(echo "$Q" | jget "d['status']")" = 200 ] || die "quote failed"
COMPANY0=$(curl -s -H "Authorization: Bearer $WALLET_API_TOKEN" "http://127.0.0.1:7979/v1/accounts/$ORG" | jget "d['total_zats']")
BEFORE=$(LV show); log "accounting BEFORE: $BEFORE"; log "company account total before: $COMPANY0"

log "=== 4: ONE withdrawal; retries must return it"
KEY=$(python -c "import uuid;print(uuid.uuid4())")
R=$(LV withdraw "$AMOUNT_ZEC" "$KEY"); log "withdraw: $R"
[ "$(echo "$R" | jget "d['status']")" = 201 ] || die "withdrawal not created"
P1=$(echo "$R" | jget "d['payout']['id']")
[ "$(echo "$R" | jget "d['payout']['type']")" = vesting ] || die "payout is not a vesting payout"
R=$(LV withdraw "$AMOUNT_ZEC" "$KEY"); log "retry, same key: $R"
[ "$(echo "$R" | jget "d['payout']['id']")" = "$P1" ] && [ "$(echo "$R" | jget "d['duplicate']")" = True ] || die "same-key retry did not return the original payout"
R=$(LV withdraw "$AMOUNT_ZEC"); log "new key while in flight: $R"
[ "$(echo "$R" | jget "d['payout']['id']")" = "$P1" ] || die "a second withdrawal started while the first was unfinished"
log "after withdraw: $(LV show)"

log "=== 5: restart this run's payroll-api while the payout confirms"
LV wait "$P1" submitted 900 | tee -a "$LOG"
hard_kill "$API_PID"; sleep 3
[ -z "$(port_pid 7980)" ] || die "payroll-api still listening"
log "payroll-api stopped (pid $API_PID)"
start_api
log "payroll-api started again (pid $API_PID)"

log "=== 6: confirmations"
LV wait "$P1" confirmed 3600 | tee -a "$LOG"
AFTER=$(LV show); log "accounting AFTER: $AFTER"
python - "$BEFORE" "$AFTER" "$AMOUNT" "$P1" <<'PY' | tee -a "$LOG" || die "accounting checks failed"
import json, sys
b, a, amt, pid = json.loads(sys.argv[1]), json.loads(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
B, A = b["balance"], a["balance"]
for x in (B, A):
    assert x["vested_zats"] + x["unvested_zats"] == x["total_zats"], x
    assert x["paid_zats"] + x["reserved_zats"] + x["available_zats"] == x["vested_zats"], x
assert B["paid_zats"] == 0 and B["reserved_zats"] == 0, B
assert A["paid_zats"] == amt and A["reserved_zats"] == 0, A
assert [p["id"] for p in a["payouts"]] == [pid] and a["payouts"][0]["state"] == "confirmed", a["payouts"]
assert a["payouts"][0]["type"] == "vesting"
cl = [c for c in a["claims"] if c["payout"] == pid]
assert len(cl) == 1 and cl[0]["status"] == "settled" and cl[0]["amount_zats"] == amt, a["claims"]
assert [c for c in a["claims"] if c["status"] == "reserved"] == [], a["claims"]
print("accounting: vested = paid + reserved + available, total = vested + unvested; one settled claim of", amt, "zats; one confirmed vesting payout")
PY
P=$(echo "$AFTER" | python -c "import json,sys; print(json.dumps(json.load(sys.stdin)['payouts'][0]))")
TX=$(echo "$P" | jget "d['txid']"); FEE=$(echo "$P" | jget "d['fee_zats']"); FEST=$(echo "$P" | jget "d['fee_estimate_zats']")
log "txid $TX fee $FEE (estimate $FEST) mined $(echo "$P" | jget "d['mined_height']") confirmations $(echo "$P" | jget "d['confirmations']")/$CONF"
"$CLI" sync carol >/dev/null 2>&1 || "$CLI" sync carol >/dev/null
"$CLI" history carol | python -c "
import sys, json
h = json.load(sys.stdin); memo = 'hushush payout ' + sys.argv[1]
hits = [(t['txid'], o['value_zats']) for t in h for o in t['outputs'] if not o['is_sent'] and o['memo'] == memo]
assert hits == [(sys.argv[2], int(sys.argv[3]))], hits
print('carol received exactly once:', hits[0][1], 'zats in', sys.argv[2])
" "$P1" "$TX" "$AMOUNT" | tee -a "$LOG" || die "carol receipt check failed"
log "carol tx status (lightwalletd, independent of the app): $("$CLI" status carol "$TX")"
N_TX=$(memo_count treasury "$P1"); log "treasury-wallet transactions carrying payout $P1's memo: $N_TX"
[ "$N_TX" = 1 ] || die "payout has $N_TX transactions"
CAROL_AFTER=$("$CLI" balance carol | jget "d['total']")
log "carol total $CAROL_BEFORE → $CAROL_AFTER (delta $((CAROL_AFTER - CAROL_BEFORE)), expected $AMOUNT)"
[ $((CAROL_AFTER - CAROL_BEFORE)) -eq "$AMOUNT" ] || die "carol delta wrong"
sleep 25
COMPANY1=$(curl -s -H "Authorization: Bearer $WALLET_API_TOKEN" "http://127.0.0.1:7979/v1/accounts/$ORG" | jget "d['total_zats']")
log "company account total $COMPANY0 → $COMPANY1 (delta $((COMPANY0 - COMPANY1)), expected $((AMOUNT + FEE)) = withdrawal + fee)"
[ $((COMPANY0 - COMPANY1)) -eq $((AMOUNT + FEE)) ] || die "company account accounting wrong"
FAUCET_FEE=$(curl -s -H "Authorization: Bearer $WALLET_API_TOKEN" "http://127.0.0.1:7979/v1/payments/$FAUCET_ID" | jget "d['fee_zats']")
TREAS1=$(wstatus | jget "d['total_zats']")
log "treasury total $TREAS0 → $TREAS1 (delta $((TREAS0 - TREAS1)), expected $((FAUCET_AMT + FAUCET_FEE)) = faucet top-up + its fee)"
[ $((TREAS0 - TREAS1)) -eq $((FAUCET_AMT + FAUCET_FEE)) ] || die "treasury accounting wrong"
log "pages: $(LV pages "$CAROL" "$TX")"
log "faucet txid: $(curl -s -H "Authorization: Bearer $WALLET_API_TOKEN" "http://127.0.0.1:7979/v1/payments/$FAUCET_ID" | jget "d['txid']")"
log "paid txid: $TX"
log "PASS: vesting withdrawal live on testnet — agreement activated, vested balance withdrawn once, confirmed ($CONF), claim settled once, payroll-api restart mid-confirmation"
