#!/usr/bin/env bash
# Writes every variable the web app needs on Vercel into ONE local file,
# outside the repository, in .env format (paste it into Vercel → Project →
# Settings → Environment Variables). Nothing is printed to the terminal.
#
#   bash deploy/vercel-vars.sh https://hushush.vercel.app [https://hushush-production.up.railway.app]
#
# APP_URL is the site's final public address: passkeys are bound to its host,
# so it must not change after real users sign up.
#
# Reads (outside the repo, in %LOCALAPPDATA%/hushush-dev):
#   deploy.env        DATABASE_URL (Supabase session pooler), PAYROLL_API_TOKEN,
#                     SMTP_URL, MAIL_FROM
#   supabase-ca.crt   the database CA certificate
# Writes:
#   vercel.env        delete it after pasting into Vercel
set -euo pipefail

APP_URL=${1:?usage: bash deploy/vercel-vars.sh <APP_URL, e.g. https://hushush.vercel.app> [PAYROLL_API_URL]}
API_URL=${2:-https://hushush-production.up.railway.app}
case "$APP_URL" in https://*) ;; *) echo "APP_URL must start with https://" >&2; exit 1 ;; esac
APP_URL=${APP_URL%/}
DEV="${LOCALAPPDATA:?run this on the Windows machine that holds deploy.env}/hushush-dev"
OUT="$DEV/vercel.env"
for f in "$DEV/deploy.env" "$DEV/supabase-ca.crt"; do [ -f "$f" ] || { echo "missing: $f" >&2; exit 1; }; done

get() { grep -m1 "^$1=" "$DEV/deploy.env" | cut -d= -f2- | sed -e 's/^"//' -e 's/"$//' | tr -d '\r'; }
DB_URL=$(get DATABASE_URL)
API_TOKEN=$(get PAYROLL_API_TOKEN)
SMTP=$(get SMTP_URL)
FROM=$(get MAIL_FROM)
[ -n "$DB_URL" ] && [ -n "$API_TOKEN" ] || { echo "deploy.env lacks DATABASE_URL or PAYROLL_API_TOKEN" >&2; exit 1; }
CA_ONE_LINE=$(tr -d '\r' <"$DEV/supabase-ca.crt" | awk 'NF' | paste -sd'|' - | sed 's/|/\\n/g')

umask 077
{
  echo "APP_URL=$APP_URL"
  echo "PAYROLL_API_URL=$API_URL"
  echo "PAYROLL_API_TOKEN=$API_TOKEN"
  echo "DATABASE_URL=$DB_URL"
  echo "DATABASE_CA_PEM=$CA_ONE_LINE"
  echo "DATABASE_POOL_MAX=2"
  [ -n "$SMTP" ] && echo "SMTP_URL=$SMTP"
  [ -n "$FROM" ] && echo "MAIL_FROM=$FROM"
} >"$OUT"
echo "wrote $OUT ($(grep -c '=' "$OUT") variables). Paste it into Vercel → Settings → Environment Variables, then delete it."
