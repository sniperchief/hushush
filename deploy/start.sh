#!/usr/bin/env bash
# hushush backend container (Zcash TESTNET ONLY).
#
#   payout-wallet  127.0.0.1:7979   private: the only process that can spend
#   payroll-api    0.0.0.0:$PORT    public behind the platform's TLS edge,
#                                   protected by PAYROLL_API_TOKEN
#
# Wallet secrets arrive as two separate variables; neither alone can spend:
#   WALLET_AGE_IDENTITY       the age identity line (AGE-SECRET-KEY-1…)
#   WALLET_MNEMONIC_AGE_B64   base64 of the wallet's mnemonic.age (encrypted seed)
# The identity is written to memory (/dev/shm), never to the volume. The
# encrypted seed, the wallet database and the payment-intent store live on the
# volume under $HUSHUSH_DATA_DIR. On the first start the wallet database is
# created by `hushush-wallet-proof restore`, with the seed decrypted in memory
# and piped in; the plaintext seed is never written to disk or logged.
set -euo pipefail

: "${WALLET_NAME:?set WALLET_NAME}"
: "${WALLET_AGE_IDENTITY:?set WALLET_AGE_IDENTITY}"
: "${WALLET_MNEMONIC_AGE_B64:?set WALLET_MNEMONIC_AGE_B64}"
: "${WALLET_BIRTHDAY:?set WALLET_BIRTHDAY (the wallet birthday height)}"
: "${WALLET_LIGHTWALLETD_URL:?set WALLET_LIGHTWALLETD_URL}"
: "${WALLET_API_TOKEN:?set WALLET_API_TOKEN}"

export HUSHUSH_DATA_DIR="${HUSHUSH_DATA_DIR:-/data/hushush}"
MEM=/dev/shm
[ -d "$MEM" ] && [ -w "$MEM" ] || MEM=/tmp
export HUSHUSH_SECRETS_DIR="$MEM/hushush-secrets"
# The CLI reads its lightwalletd server from here.
export HUSHUSH_LIGHTWALLETD="$WALLET_LIGHTWALLETD_URL"

umask 077
WDIR="$HUSHUSH_DATA_DIR/wallets/$WALLET_NAME"
ID="$HUSHUSH_SECRETS_DIR/$WALLET_NAME.age-identity.txt"
mkdir -p "$HUSHUSH_SECRETS_DIR" "$WDIR"

printf '# hushush wallet identity (from WALLET_AGE_IDENTITY)\n%s\n' "$WALLET_AGE_IDENTITY" >"$ID"
printf '%s' "$WALLET_MNEMONIC_AGE_B64" | base64 -d >"$WDIR/mnemonic.age.new"
if ! age -d -i "$ID" "$WDIR/mnemonic.age.new" >/dev/null 2>&1; then
  echo "start: WALLET_AGE_IDENTITY does not decrypt WALLET_MNEMONIC_AGE_B64; refusing to start" >&2
  rm -f "$WDIR/mnemonic.age.new"
  exit 1
fi
mv -f "$WDIR/mnemonic.age.new" "$WDIR/mnemonic.age"

if [ ! -f "$WDIR/data.sqlite" ]; then
  echo "start: first start for wallet '$WALLET_NAME': creating its database from the seed (birthday $WALLET_BIRTHDAY)"
  SCRATCH=$(mktemp -d "$MEM/restore.XXXXXX")
  trap 'rm -rf "$SCRATCH"' EXIT
  age -d -i "$ID" "$WDIR/mnemonic.age" |
    HUSHUSH_DATA_DIR="$SCRATCH" HUSHUSH_SECRETS_DIR="$SCRATCH/secrets" \
      hushush-wallet-proof restore "$WALLET_NAME" --mnemonic-file - --birthday "$WALLET_BIRTHDAY" >/dev/null
  # Keep only the database (viewing keys and sync state). The scratch copy of
  # the seed and its throwaway identity are deleted with $SCRATCH.
  cp "$SCRATCH/wallets/$WALLET_NAME"/data.sqlite* "$WDIR/"
  rm -rf "$SCRATCH"
  trap - EXIT
  echo "start: wallet database created; it syncs from the birthday on start"
fi

hushush-payout-wallet &
WALLET_PID=$!

PAYROLL_API_ADDR="0.0.0.0:${PORT:-8080}" PAYROLL_API_ALLOW_NONLOCAL=1 WALLET_URL="http://127.0.0.1:7979" \
  hushush-payroll-api &
API_PID=$!

stop() {
  kill -TERM "$API_PID" "$WALLET_PID" 2>/dev/null || true
  wait "$API_PID" "$WALLET_PID" 2>/dev/null || true
}
trap 'stop; exit 0' TERM INT

# If either process exits, stop the other and exit, so the platform restarts
# the container (the wallet's start-up recovery then runs again).
set +e
wait -n "$WALLET_PID" "$API_PID"
code=$?
set -e
echo "start: a process exited with code $code; stopping" >&2
stop
exit "$code"
