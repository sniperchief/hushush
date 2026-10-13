#!/usr/bin/env bash
# hushush Phase 0 — live TESTNET end-to-end proof.
#
#   1. creates two wallets (payer "alice", payee "bob") if they do not exist
#   2. waits until alice has spendable shielded funds (fund her UA first!)
#   3. sends PAY_ZATS from alice to bob with a memo
#   4. waits for confirmations, syncs bob, verifies amount + memo received
#   5. prints the keyless observer view of the transaction
#   6. restores alice from her mnemonic into a fresh wallet and verifies the
#      restored wallet finds the same history/balance (recovery proof)
#
# Every step is a separate process, so persistence across restarts is exercised
# throughout. Output is appended to $LOG (no secrets are ever printed).
set -euo pipefail
export RUST_LOG=${RUST_LOG:-warn}

BIN=${BIN:-"$(dirname "$0")/../target/debug/hushush-wallet-proof"}
PAY_ZATS=${PAY_ZATS:-150000}
CONFIRMATIONS=${CONFIRMATIONS:-3}
MEMO=${MEMO:-"hushush phase0 testnet payout $(date -u +%Y%m%dT%H%M%SZ)"}
RUN_ID=${RUN_ID:-$(date -u +%Y%m%d%H%M%S)}
mkdir -p "$(dirname "$0")/../evidence"
LOG=${LOG:-"$(dirname "$0")/../evidence/e2e-run-$RUN_ID.log"}
w() { "$BIN" "$@"; }
# Retry transient network failures (public lightwalletd occasionally drops TLS).
wr() { local i out; for i in 1 2 3 4 5; do if out=$("$BIN" "$@" 2>>"$LOG"); then printf '%s
' "$out"; return 0; fi; log "retry $i: $*"; sleep 20; done; return 1; }
log() { echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$LOG"; }
jget() { python -c "import sys,json;print(json.load(sys.stdin)$1)"; }

log "server: $(w info)"
for n in alice bob; do
  if ! w address "$n" >/dev/null 2>&1; then log "creating wallet $n"; w init "$n" >/dev/null; fi
  log "$n address: $(w address "$n")"
done

log "waiting for alice to have >= $((PAY_ZATS + 20000)) spendable zats (fund: $(w address alice))"
while true; do
  spendable=$(wr sync alice | jget "['spendable_total']")
  log "alice spendable=$spendable"
  [ "$spendable" -ge $((PAY_ZATS + 20000)) ] && break
  sleep 60
done

bob_before=$(wr sync bob | jget "['total']")
log "bob total before: $bob_before"

TXID=$(w send alice --to "$(w address bob)" --zats "$PAY_ZATS" --memo "$MEMO" 2>>"$LOG")
log "broadcast txid: $TXID"

while true; do
  wr sync alice >/dev/null
  conf=$(w status alice "$TXID" | jget "['confirmations']")
  log "confirmations: $conf"
  [ "$conf" -ge "$CONFIRMATIONS" ] && break
  sleep 45
done

bob_after=$(wr sync bob | jget "['total']")
log "bob total after: $bob_after (delta $((bob_after - bob_before)))"
[ $((bob_after - bob_before)) -eq "$PAY_ZATS" ] || { log "FAIL: bob delta != $PAY_ZATS"; exit 1; }
w history bob | python -c "
import sys,json
h=json.load(sys.stdin); txid=sys.argv[1]; memo=sys.argv[2]
t=[x for x in h if x['txid']==txid]
assert t, 'bob has no record of tx'
outs=[o for o in t[0]['outputs'] if not o['is_sent']]
assert any(o['value_zats']==int(sys.argv[3]) and o['memo']==memo for o in outs), outs
print('bob received exact amount with memo in pool(s):', sorted({o['pool'] for o in outs}))
" "$TXID" "$MEMO" "$PAY_ZATS" | tee -a "$LOG"

log "observer view:"
w observe "$TXID" | tee -a "$LOG"

log "recovery: restoring alice from mnemonic into alice-restored-$RUN_ID"
TMP=$(mktemp -d)
w backup-mnemonic alice --out "$TMP/m.txt"
BDAY=$(w birthday alice)
w restore "alice-restored-$RUN_ID" --mnemonic-file "$TMP/m.txt" --birthday "$BDAY" >/dev/null
rm -f "$TMP/m.txt"; rmdir "$TMP"
orig=$(wr sync alice | jget "['total']")
rest=$(wr sync "alice-restored-$RUN_ID" | jget "['total']")
log "alice total=$orig restored total=$rest"
[ "$orig" = "$rest" ] || { log "FAIL: restored balance differs"; exit 1; }
[ "$(w address alice)" = "$(w address "alice-restored-$RUN_ID")" ] || { log "FAIL: address differs"; exit 1; }
log "PASS: end-to-end shielded payment, confirmation, observer check and recovery"
