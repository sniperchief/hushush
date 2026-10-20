#!/usr/bin/env bash
# hushush LIVE TESTNET fault-injection run for VESTING withdrawals (Vesting Phase 4).
#
# Real Next.js app, payroll-api, PostgreSQL (a NEW database per run), payout
# wallet on `treasury`, lightwalletd on the NU7 testnet. Wallet-side crashes
# use the test-only build (`--features fault-injection`, copied to
# hushush-payout-wallet-fault.exe): it exits with code 86 at WALLET_FAULT.
# One company (one faucet top-up), one employee, one agreement per scenario
# (1 ZEC over 1 day, no cliff), each withdrawing AMOUNT to carol:
#
#   A  crash_before_build        intent pending, nothing built. payroll-api is
#                                restarted while the wallet is down (claim must
#                                stay reserved), then the wallet restarts →
#                                intent failed → payout failed, claim released
#                                once, 0 transactions; a late submit of the same
#                                payment id builds nothing (D: proven failure)
#   B1 crash_after_wallet_store  signed tx only in the wallet DB → recovered by
#                                memo → same tx broadcast → confirmed
#   B2 crash_after_build         intent built, never broadcast (node: not
#                                found); payout stays requires_reconciliation →
#                                restart → same bytes broadcast → confirmed
#   C  crash_after_broadcast     network accepted, response lost; payout stays
#                                requires_reconciliation (never failed) while
#                                the wallet is down; the agreement is CANCELLED
#                                meanwhile (E2) → restart → confirmed, settled
#   E  normal wallet             withdrawal submitted, agreement CANCELLED while
#                                in flight → confirmed, settled; vesting frozen
#
# Before the crash_after_broadcast wallet starts, every earlier transaction must
# be mined (its start-up rebroadcast would otherwise hit the fault point).
#
# TESTNET ONLY. Requires LIVE_AUTHORIZED=testnet. Ports 3300/7980/7979 free.
set -euo pipefail
[ "${LIVE_AUTHORIZED:-}" = testnet ] || { echo "refusing: set LIVE_AUTHORIZED=testnet to broadcast on Zcash TESTNET" >&2; exit 2; }

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
D="$LOCALAPPDATA/hushush-dev"
RUN_ID=${RUN_ID:-v4-$(date -u +%Y%m%d%H%M%S)}
EVID="$ROOT/payout-wallet/evidence"
LOG="$EVID/live-$RUN_ID.log"
SVC="$EVID/live-$RUN_ID.services.log"
# The wallet logs to its own file: concurrent appends to one file lose lines on Windows.
WSVC="$EVID/live-$RUN_ID.wallet.log"
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
memo_count() { # wallet name, payout id -> number of transactions carrying that payout's memo
  "$CLI" history "$1" | python -c "
import sys, json
h = json.load(sys.stdin); memo = 'hushush payout ' + sys.argv[1]
print(len({t['txid'] for t in h for o in t['outputs'] if o['memo'] == memo}))" "$2"
}
intent_of() { # payout id -> the wallet's intent row (no raw bytes)
  python - "$INTENTS" "$1" <<'EOF'
import sqlite3, sys, json
c = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
r = c.execute("SELECT state, txid, broadcast_attempts, failure_reason, fee_zats, raw_tx IS NOT NULL FROM payment_intents WHERE payment_id = ?", (sys.argv[2],)).fetchone()
print(json.dumps({"state": r[0], "txid": r[1], "broadcast_attempts": r[2], "failure_reason": r[3], "fee_zats": r[4], "raw_persisted": bool(r[5])} if r else None))
EOF
}
dbq() { # SQL -> JSON rows from this run's database
  (cd "$ROOT/web" && DATABASE_URL="$DB_URL" node -e "
const pg=require('pg');const c=new pg.Client({connectionString:process.env.DATABASE_URL});
c.connect().then(()=>c.query(process.argv[1])).then(r=>{console.log(JSON.stringify(r.rows));return c.end()})" "$1")
}
events_of() { dbq "select to_state from payout_events where payout_id = '$1' order by id" | python -c "import sys,json; print(' > '.join(r['to_state'] for r in json.load(sys.stdin)))"; }
pstate() { LA "$1" show | python -c "import sys,json; d=json.load(sys.stdin); p=d['payouts'][0]; c=d['claims'][0]; print(json.dumps({'payout': p['state'], 'confirmations': p['confirmations'], 'txid': p['txid'], 'claim': c['status'], 'reserved': d['balance']['reserved_zats'], 'paid': d['balance']['paid_zats'], 'available': d['balance']['available_zats'], 'detail': p['detail']}))"; }

start_wallet() { # $1 = binary, rest = extra env
  local bin=$1; shift
  env "$@" WALLET_API_TOKEN="$WALLET_API_TOKEN" WALLET_NAME=treasury WALLET_LIGHTWALLETD_URL="$LWD" WALLET_SYNC_SECS=20 "$bin" >>"$WSVC" 2>&1 &
  WALLET_PID=$!
  wait_http http://127.0.0.1:7979/healthz 600
}
stop_wallet() { kill "$WALLET_PID" 2>/dev/null || hard_kill "$WALLET_PID"; wait "$WALLET_PID" 2>/dev/null || true; sleep 2; [ -z "$(port_pid 7979)" ] || die "wallet still listening"; }
start_api() {
  PAYROLL_DATABASE_URL="$DB_URL" PAYROLL_API_ADDR=127.0.0.1:7980 PAYROLL_API_TOKEN="$PAYROLL_API_TOKEN" \
  WALLET_URL=http://127.0.0.1:7979 WALLET_API_TOKEN="$WALLET_API_TOKEN" PAYOUT_ENABLED_ORGS='*' PAYOUT_CONFIRMATIONS="$CONF" \
  PAYOUT_ADDRESS_KEY="$PAYOUT_ADDRESS_KEY" PAYOUT_POLL_SECS=15 WALLET_SUBMIT_TIMEOUT_SECS=180 "$API_BIN" >>"$SVC" 2>&1 &
  API_PID=$!
  wait_http http://127.0.0.1:7980/healthz
}
wait_spendable() { # agreement label: until a quote for AMOUNT is sufficient
  for _ in $(seq 1 90); do
    local q; q=$(LA "$1" quote "$AMOUNT_ZEC")
    [ "$(echo "$q" | jget "d.get('sufficient')")" = True ] && { log "quote $1: fee $(echo "$q" | jget "d['fee_zats']") sufficient"; return 0; }
    log "waiting for spendable company funds: $(echo "$q" | jget "d.get('reason') or d.get('message')")"; sleep 20
  done
  die "company account never became spendable"
}
wait_mined() { # label
  for _ in $(seq 1 120); do
    local s; s=$(pstate "$1")
    [ "$(echo "$s" | jget "d['confirmations'] >= 1")" = True ] && { log "$1 mined: $s"; return 0; }
    sleep 15
  done
  die "$1 never mined"
}
# Withdraw on agreement $1 through a wallet that crashes at fault point $2; prints nothing, sets FP (payout id).
fault_withdraw() {
  wait_spendable "$1"
  stop_wallet
  start_wallet "$FAULT_BIN" WALLET_FAULT="$2"; log "fault-injection wallet started (pid $WALLET_PID, WALLET_FAULT=$2)"
  local r; r=$(LA "$1" withdraw "$AMOUNT_ZEC"); log "withdraw $1: $r"
  [ "$(echo "$r" | jget "d['status']")" = 201 ] || die "withdraw $1 not created"
  [ "$(echo "$r" | jget "d['payout']['state']")" = requires_reconciliation ] || die "withdraw $1 did not reach requires_reconciliation"
  set +e; wait "$WALLET_PID"; local code=$?; set -e
  log "fault wallet exit code: $code (86 = injected crash at $2)"
  [ "$code" = 86 ] || die "fault point $2 did not fire"
  FP=$(echo "$r" | jget "d['payout']['id']")
  log "wallet intent $1 after crash: $(intent_of "$FP")"
  log "payroll view $1 after crash: $(pstate "$1")"
}
hold_check() { # label, seconds: while the wallet is down nothing may resolve
  sleep "$2"
  local s; s=$(pstate "$1"); log "$1 after ${2}s with the wallet down: $s"
  echo "$s" | python -c "import sys,json; s=json.load(sys.stdin); assert s['payout']=='requires_reconciliation' and s['claim']=='reserved' and s['paid']==0, s" || die "$1 resolved while the wallet was down"
}

for port in 3300 7980 7979; do
  [ -z "$(port_pid $port)" ] || { echo "port $port is already in use; refusing to start" >&2; exit 1; }
done
[ -f "$FAULT_BIN" ] || die "fault binary missing"
powershell -NoProfile -Command "Add-Type -Name P -Namespace W -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint f);'; [void][W.P]::SetThreadExecutionState(0x80000001); while (\$true) { Start-Sleep 60 }" >/dev/null 2>&1 &
AWAKE_PID=$!

log "=== hushush Vesting Phase 4 live fault-injection run $RUN_ID (TESTNET, $LWD, $CONF confirmations, $AMOUNT_ZEC ZEC per withdrawal)"
log "binaries: wallet $(sha256sum "$WALLET_BIN" | cut -c1-16) fault $(sha256sum "$FAULT_BIN" | cut -c1-16) payroll-api $(sha256sum "$API_BIN" | cut -c1-16)"
INFO=$("$CLI" info); log "lightwalletd: $INFO"
case "$INFO" in *" chain=test "*"branch_id=77190ad9"*) log "network check: chain=test, NU7 testnet branch 77190ad9" ;; *) die "lightwalletd is not the NU7 testnet" ;; esac
(cd "$ROOT/web" && node -e "
const pg=require('pg');const c=new pg.Client({connectionString:process.env.DATABASE_URL});
c.connect().then(()=>c.query('CREATE DATABASE $DB_NAME')).then(()=>c.end())")
log "database $DB_NAME created"

start_wallet "$WALLET_BIN"
start_api
(cd "$ROOT/web" && DATABASE_URL="$DB_URL" npm run db:migrate >/dev/null && DATABASE_URL="$DB_URL" PAYROLL_API_URL=http://127.0.0.1:7980 PAYROLL_API_TOKEN="$PAYROLL_API_TOKEN" npx next start -p 3300 >>"$SVC" 2>&1) &
WEB_PID=$!
wait_http http://localhost:3300/login
log "services up: wallet pid $WALLET_PID, payroll-api pid $API_PID, web pid $WEB_PID"
log "payroll schema: $(dbq "select max(version::int) as v from payroll_schema_migrations")"

CAROL=$("$CLI" address carol)
CAROL_BEFORE=$("$CLI" sync carol 2>/dev/null | jget "d['total']")
TREAS0=$(wstatus | jget "d['total_zats']")
log "treasury total=$TREAS0; carol total=$CAROL_BEFORE"
AMOUNT=$(python -c "from decimal import Decimal; print(int(Decimal('$AMOUNT_ZEC')*100000000))")

log "=== setup: company, faucet, employee, five agreements"
SETUP=$(LV setup 1 90); log "setup (agreement E): $SETUP"
FAUCET_ID=$(echo "$SETUP" | jget "d['faucet']['payment_id']"); FAUCET_AMT=$(echo "$SETUP" | jget "d['faucet']['amount_zats']")
ORG=$(echo "$SETUP" | jget "d['organizationId']")
[ "$(echo "$SETUP" | jget "d['faucet']['state']")" = broadcast ] || die "faucet top-up was not broadcast"
python - "$STATE_FILE" <<'EOF'
import json, sys
s = json.load(open(sys.argv[1])); s.setdefault("agreements", {})["E"] = s["agreementId"]; json.dump(s, open(sys.argv[1], "w"))
EOF
for L in A B1 B2 C; do log "agreement $L: $(LV create "$L" 1 90)"; done
for L in A B1 B2 C E; do
  R=$(LA "$L" address "$CAROL"); [ "$(echo "$R" | jget "d['address_locked']")" = True ] || die "address not locked on $L"
done
log "carol's address locked on all five agreements"
log "company wallet: $(LV wait-funded $((AMOUNT * 4 + 50000)) 1500)"
for L in A B1 B2 C E; do LA "$L" wait-available "$AMOUNT" 600 >/dev/null; done
COMPANY0=$(wcurl "http://127.0.0.1:7979/v1/accounts/$ORG" | jget "d['total_zats']")
log "all agreements have $AMOUNT available; company account total $COMPANY0"

# ------------------------------------------------------------------ A / D
log "=== A: wallet crashes after the durable intent, BEFORE building (proven failure → release)"
fault_withdraw A crash_before_build; PA=$FP
hard_kill "$API_PID"; sleep 2; start_api; log "payroll-api restarted while the wallet is down (pid $API_PID)"
hold_check A 45
start_wallet "$WALLET_BIN"; log "normal wallet restarted (pid $WALLET_PID): start-up recovery"
log "wallet intent A after recovery: $(intent_of "$PA")"
LA A wait "$PA" failed 900 | tee -a "$LOG"
SA=$(pstate A); log "A final: $SA"
echo "$SA" | python -c "import sys,json; s=json.load(sys.stdin); assert s['claim']=='released' and s['reserved']==0 and s['paid']==0 and s['txid'] is None, s" || die "A not released cleanly"
LATE=$(wcurl -X POST -H 'content-type: application/json' http://127.0.0.1:7979/v1/payments \
  -d "{\"payment_id\":\"$PA\",\"account\":\"$ORG\",\"address\":\"$CAROL\",\"amount_zats\":$AMOUNT,\"max_fee_zats\":$((10000 + 10000))}")
log "late submit of A's payment id directly to the wallet: $(echo "$LATE" | python -c "import sys,json; d=json.load(sys.stdin); print({k: d.get(k) for k in ('state','txid','failure_reason','error','message')})")"
log "wallet intent A after late submit: $(intent_of "$PA")"
[ "$(intent_of "$PA" | jget "d['state']")" = failed ] || die "A's intent changed after the late submit"
KEY_A=$(dbq "select idempotency_key from vesting_claims where id = (select vesting_claim_id from payouts where id = '$PA')" | jget "d[0]['idempotency_key']")
R=$(LA A withdraw "$AMOUNT_ZEC" "$KEY_A"); log "A retry with its original key: $R"
[ "$(echo "$R" | jget "d['payout']['id']")" = "$PA" ] && [ "$(echo "$R" | jget "d['payout']['state']")" = failed ] || die "A's original key did not return the failed payout"
[ "$(memo_count treasury "$PA")" = 0 ] || die "a transaction exists for failed payout A"

# ------------------------------------------------------------------ B1
log "=== B1: wallet crashes after the wallet DB stored the signed tx, before the intent recorded it"
fault_withdraw B1 crash_after_wallet_store; PB1=$FP
hold_check B1 30
start_wallet "$WALLET_BIN"; log "normal wallet restarted (pid $WALLET_PID): recovery by memo"
log "wallet intent B1 after recovery: $(intent_of "$PB1")"
wait_mined B1

# ------------------------------------------------------------------ B2
log "=== B2: wallet crashes after recording the built tx, BEFORE broadcasting"
fault_withdraw B2 crash_after_build; PB2=$FP
TXB2=$(intent_of "$PB2" | jget "d['txid']")
log "chain view of B2 before restart: $("$CLI" observe "$TXB2" 2>&1 | head -c 160 | tr '\n' ' ')"
hold_check B2 30
start_wallet "$WALLET_BIN"; log "normal wallet restarted (pid $WALLET_PID): rebroadcast of the stored bytes"
log "wallet intent B2 after recovery: $(intent_of "$PB2")"
wait_mined B2

# ------------------------------------------------------------------ C + E2
log "=== C: wallet crashes right AFTER the network accepted the tx (response lost); cancellation meanwhile"
fault_withdraw C crash_after_broadcast; PC=$FP
TXC=$(intent_of "$PC" | jget "d['txid']")
log "chain view of C while the wallet is down: $("$CLI" observe "$TXC" 2>&1 | head -c 160 | tr '\n' ' ')"
hold_check C 30
R=$(LA C cancel); log "employer cancels agreement C while its payout is unresolved: $R"
[ "$(echo "$R" | jget "d['status']")" = 200 ] && [ "$(echo "$R" | jget "d['agreement_status']")" = cancelled ] || die "cancel C failed"
hold_check C 20
start_wallet "$WALLET_BIN"; log "normal wallet restarted (pid $WALLET_PID): rebroadcast of the same bytes"
log "wallet intent C after recovery: $(intent_of "$PC")"
wait_mined C

# ------------------------------------------------------------------ E
log "=== E: normal wallet; agreement cancelled while the payout is in flight"
wait_spendable E
R=$(LA E withdraw "$AMOUNT_ZEC"); log "withdraw E: $R"
[ "$(echo "$R" | jget "d['payout']['state']")" = submitted ] || die "E not submitted"
PE=$(echo "$R" | jget "d['payout']['id']")
R=$(LA E cancel); log "employer cancels agreement E while its payout is submitted: $R"
[ "$(echo "$R" | jget "d['agreement_status']")" = cancelled ] || die "cancel E failed"
log "E right after cancel: $(pstate E)"
R=$(LA E withdraw "$AMOUNT_ZEC"); log "withdraw on E again while in flight: $R"
[ "$(echo "$R" | jget "d['payout']['id']")" = "$PE" ] || die "a second E withdrawal started"

# ------------------------------------------------------------------ confirmations
log "=== confirmations"
for L in B1 B2 C E; do
  eval P=\$P$L
  LA "$L" wait "$P" confirmed 3600 | tee -a "$LOG"
done

# ------------------------------------------------------------------ verification
log "=== verification"
for L in A B1 B2 C E; do
  eval P=\$P$L
  log "$L payout $P history: $(events_of "$P")"
  log "$L agreement: $(LA "$L" show)"
done
python - "$AMOUNT" "$(cygpath -w "$ROOT/web")" <<'PY' | tee -a "$LOG" || die "accounting checks failed"
import json, subprocess, sys, os
amt, web = int(sys.argv[1]), sys.argv[2]  # a Windows path: Python here is a Windows program
def show(l):
    return json.loads(subprocess.run(["node", "scripts/live-vesting.mjs", "show"], cwd=web, env={**os.environ, "AGREEMENT": l}, capture_output=True, text=True, check=True).stdout)
for l in ["A", "B1", "B2", "C", "E"]:
    d = show(l); b = d["balance"]
    assert b["vested_zats"] + b["unvested_zats"] == b["total_zats"], (l, b)
    assert b["paid_zats"] + b["reserved_zats"] + b["available_zats"] == b["vested_zats"], (l, b)
    assert b["reserved_zats"] == 0, (l, b)
    assert len(d["payouts"]) == 1 and len(d["claims"]) == 1, (l, d)
    p, c = d["payouts"][0], d["claims"][0]
    if l == "A":
        assert p["state"] == "failed" and c["status"] == "released" and b["paid_zats"] == 0 and p["txid"] is None, d
    else:
        assert p["state"] == "confirmed" and c["status"] == "settled" and b["paid_zats"] == amt, d
    if l in ("C", "E"):
        assert d["status"] == "cancelled" and d["cancelled_at"], d
    print(f"{l}: payout {p['state']}, claim {c['status']}, vested {b['vested_zats']} = paid {b['paid_zats']} + reserved 0 + available {b['available_zats']}; unvested {b['unvested_zats']}")
PY
# Cancelled agreements stop vesting: their vested amount must not grow.
VC1=$(LA C show | jget "d['balance']['vested_zats']"); VE1=$(LA E show | jget "d['balance']['vested_zats']")
sleep 30
VC2=$(LA C show | jget "d['balance']['vested_zats']"); VE2=$(LA E show | jget "d['balance']['vested_zats']")
log "vested after cancellation, 30 s apart: C $VC1 → $VC2, E $VE1 → $VE2 (must not change)"
[ "$VC1" = "$VC2" ] && [ "$VE1" = "$VE2" ] || die "vesting continued after cancellation"
VA=$(LA A show | jget "d['balance']['vested_zats']"); log "(control) A, not cancelled, vested now $VA"

"$CLI" sync carol >/dev/null 2>&1 || "$CLI" sync carol >/dev/null
TOTAL_FEES=0; TXIDS=()
for L in B1 B2 C E; do
  eval P=\$P$L
  S=$(LA "$L" show); TX=$(echo "$S" | jget "d['payouts'][0]['txid']"); FEE=$(echo "$S" | jget "d['payouts'][0]['fee_zats']")
  TOTAL_FEES=$((TOTAL_FEES + FEE)); TXIDS+=("$TX")
  "$CLI" history carol | python -c "
import sys, json
h = json.load(sys.stdin); memo = 'hushush payout ' + sys.argv[1]
hits = [(t['txid'], o['value_zats']) for t in h for o in t['outputs'] if not o['is_sent'] and o['memo'] == memo]
assert hits == [(sys.argv[2], int(sys.argv[3]))], hits
print(sys.argv[4], 'carol received exactly once:', hits[0][1], 'zats in', sys.argv[2], 'fee', sys.argv[5])
" "$P" "$TX" "$AMOUNT" "$L" "$FEE" | tee -a "$LOG" || die "carol receipt check failed for $L"
  N=$(memo_count treasury "$P"); log "$L: treasury transactions carrying the payout memo: $N"
  [ "$N" = 1 ] || die "$L has $N transactions"
done
N=$(memo_count treasury "$PA"); log "A: treasury transactions carrying the payout memo: $N"
[ "$N" = 0 ] || die "a transaction exists for failed payout A"
CAROL_AFTER=$("$CLI" balance carol | jget "d['total']")
log "carol total $CAROL_BEFORE → $CAROL_AFTER (delta $((CAROL_AFTER - CAROL_BEFORE)), expected $((AMOUNT * 4)))"
[ $((CAROL_AFTER - CAROL_BEFORE)) -eq $((AMOUNT * 4)) ] || die "carol delta wrong"
sleep 25
COMPANY1=$(wcurl "http://127.0.0.1:7979/v1/accounts/$ORG" | jget "d['total_zats']")
log "company account total $COMPANY0 → $COMPANY1 (delta $((COMPANY0 - COMPANY1)), expected $((AMOUNT * 4 + TOTAL_FEES)) = 4 withdrawals + fees)"
[ $((COMPANY0 - COMPANY1)) -eq $((AMOUNT * 4 + TOTAL_FEES)) ] || die "company account accounting wrong"
FAUCET_FEE=$(wcurl "http://127.0.0.1:7979/v1/payments/$FAUCET_ID" | jget "d['fee_zats']")
TREAS1=$(wstatus | jget "d['total_zats']")
log "treasury total $TREAS0 → $TREAS1 (delta $((TREAS0 - TREAS1)), expected $((FAUCET_AMT + FAUCET_FEE)))"
[ $((TREAS0 - TREAS1)) -eq $((FAUCET_AMT + FAUCET_FEE)) ] || die "treasury accounting wrong"
log "faucet txid: $(wcurl "http://127.0.0.1:7979/v1/payments/$FAUCET_ID" | jget "d['txid']")"
log "paid txids (B1 B2 C E): ${TXIDS[*]}"
log "payout ids: A=$PA B1=$PB1 B2=$PB2 C=$PC E=$PE"
log "PASS: vesting fault-injection run — A failed+released with 0 tx; B1, B2, C recovered the original tx; C and E cancelled in flight and settled once"
