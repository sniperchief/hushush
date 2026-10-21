#!/usr/bin/env bash
# Writes every variable the Railway backend service needs into ONE local file,
# outside the repository, in .env format (paste it into Railway's
# Variables → Raw Editor). Nothing is printed to the terminal.
#
#   bash deploy/railway-vars.sh            # wallet prod-treasury (default)
#
# Reads (all outside the repo, in %LOCALAPPDATA%):
#   hushush-dev/deploy.env           DATABASE_URL (Supabase session pooler),
#                                    PAYROLL_API_TOKEN, WALLET_API_TOKEN
#   hushush-dev/supabase-ca.crt      the database CA certificate
#   hushush-phase0/wallets/<name>/mnemonic.age, …/secrets/<name>.age-identity.txt
# Creates once and then reuses (never change it after the first deploy):
#   hushush-dev/railway-payout-address-key.txt
# Writes:
#   hushush-dev/railway.env          delete it after pasting into Railway
set -euo pipefail

NAME=${1:-prod-treasury}
LAD=${LOCALAPPDATA:?run this on the Windows machine that holds the wallet}
DEV="$LAD/hushush-dev"
WAL="$LAD/hushush-phase0/wallets/$NAME"
SEC="$LAD/hushush-phase0/secrets/$NAME.age-identity.txt"
CLI="$(cd "$(dirname "$0")/.." && pwd)/wallet-proof/target/debug/hushush-wallet-proof.exe"
OUT="$DEV/railway.env"
KEYFILE="$DEV/railway-payout-address-key.txt"

for f in "$DEV/deploy.env" "$DEV/supabase-ca.crt" "$WAL/mnemonic.age" "$SEC"; do
  [ -f "$f" ] || { echo "missing: $f" >&2; exit 1; }
done

get() { grep -m1 "^$1=" "$DEV/deploy.env" | cut -d= -f2- | sed -e 's/^"//' -e 's/"$//'; }
DB_URL=$(get DATABASE_URL)
API_TOKEN=$(get PAYROLL_API_TOKEN)
WALLET_TOKEN=$(get WALLET_API_TOKEN)
[ -n "$DB_URL" ] && [ -n "$API_TOKEN" ] && [ -n "$WALLET_TOKEN" ] || { echo "deploy.env lacks DATABASE_URL, PAYROLL_API_TOKEN or WALLET_API_TOKEN" >&2; exit 1; }

umask 077
[ -s "$KEYFILE" ] || python -c "import secrets;print(secrets.token_hex(32))" >"$KEYFILE"
ADDRESS_KEY=$(tr -d '\r\n' <"$KEYFILE")
IDENTITY=$(grep -m1 '^AGE-SECRET-KEY-' "$SEC" | tr -d '\r\n')
SEED_B64=$(base64 -w0 <"$WAL/mnemonic.age")
CA_ONE_LINE=$(tr -d '\r' <"$DEV/supabase-ca.crt" | awk 'NF' | paste -sd'|' - | sed 's/|/\\n/g')
BIRTHDAY=$("$CLI" birthday "$NAME" 2>/dev/null | tr -dc '0-9')
[ -n "$IDENTITY" ] && [ -n "$SEED_B64" ] && [ -n "$BIRTHDAY" ] || { echo "could not read the wallet's identity, seed or birthday" >&2; exit 1; }

cat >"$OUT" <<EOF
# hushush backend on Railway (Zcash TESTNET). SECRET: paste into Railway, then delete this file.
WALLET_NAME=$NAME
WALLET_BIRTHDAY=$BIRTHDAY
WALLET_LIGHTWALLETD_URL=https://testnet.zec.rocks:443
WALLET_AGE_IDENTITY=$IDENTITY
WALLET_MNEMONIC_AGE_B64=$SEED_B64
WALLET_API_TOKEN=$WALLET_TOKEN
PAYROLL_API_TOKEN=$API_TOKEN
PAYOUT_ADDRESS_KEY=$ADDRESS_KEY
PAYROLL_DATABASE_URL=$DB_URL
PAYROLL_DATABASE_CA_PEM="$CA_ONE_LINE"
PAYROLL_DB_POOL_SIZE=5
PAYOUT_ENABLED_ORGS=*
PAYOUT_CONFIRMATIONS=10
EOF
echo "wrote $OUT ($(grep -c '=' "$OUT") variables). Paste it into Railway → service → Variables → Raw Editor, then delete it."
