#!/usr/bin/env bash
# Runs every automated check in the repository and prints one line per check.
# Usage: bash docs/phase4-evidence/run-checks.sh > docs/phase4-evidence/<name>.txt
# Needs the local dev PostgreSQL (%LOCALAPPDATA%\hushush-dev\dev.env); the
# test databases it uses are wiped. wallet-proof's live tests need network.
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
export PATH="$USERPROFILE/.cargo/bin:$PATH"
set -a; . "$LOCALAPPDATA/hushush-dev/dev.env"; set +a
TEST_DB="${DATABASE_URL%/*}/hushush_test"
unset DATABASE_URL PAYROLL_API_URL PAYROLL_API_TOKEN APP_URL

run() { # name, dir, command...
  local name=$1 dir=$2; shift 2
  local out; out=$(cd "$ROOT/$dir" && "$@" 2>&1); local code=$?
  local summary
  summary=$(echo "$out" | grep -E "test result:|Tests +[0-9]|SMOKE|error(\[|:)|warning:|Diff in|problems?|found [0-9]+ vulnerab|Compiled|Finished|✓|PASS|FAIL" | sed 's/^ *//' | sort | uniq -c | tr '\n' ';' | cut -c1-600)
  printf '%-45s exit=%s  %s\n' "$name" "$code" "$summary"
}

echo "== checks $(date -u +%FT%TZ) HEAD $(git -C "$ROOT" rev-parse --short HEAD)"
for c in payroll payroll-api payout-wallet wallet-proof; do
  run "$c: cargo fmt --check" "$c" cargo fmt --check
  run "$c: cargo clippy -D warnings" "$c" cargo clippy --all-targets -- -D warnings
done
run "payroll: cargo test (SQLite+PostgreSQL)" payroll env PAYROLL_TEST_DATABASE_URL="$TEST_DB" cargo test -- --include-ignored
run "payroll-api: cargo build + test" payroll-api sh -c "cargo build && cargo test"
run "payout-wallet: cargo build + test" payout-wallet sh -c "cargo build && cargo test"
run "wallet-proof: cargo test (offline)" wallet-proof cargo test
run "wallet-proof: live testnet tests" wallet-proof cargo test --test testnet_live -- --ignored --test-threads=1
run "web: vitest (real API, MOCK wallet)" web env TEST_DATABASE_URL="$TEST_DB" npx vitest run
run "web: typecheck" web npm run typecheck
run "web: lint" web npm run lint
run "web: production build" web npm run build
