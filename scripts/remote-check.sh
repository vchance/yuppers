#!/usr/bin/env bash
# Runs the full checks for a pushed branch on a build machine over SSH, so
# that builds and tests use its disk and cores rather than the laptop's.
#
#   scripts/remote-check.sh <branch> [backend|web|e2e|all]
#
# The machine is YUPPERS_CHECK_HOST (an SSH host). It needs, in the user's
# home: a clone of the repository at ~/development/yuppers with a .env whose
# MIGRATION_DATABASE_URL and DATABASE_URL point at a Postgres the user may
# create databases in, rustup in ~/.cargo, and Node in ~/.local/opt/node.
# Each run checks the branch out in its own worktree, against its own
# database (both removed afterwards), so runs for different branches never
# share state with each other or with the clone's own database.
set -euo pipefail

branch="${1:?usage: scripts/remote-check.sh <branch> [backend|web|e2e|all]}"
what="${2:-all}"
host="${YUPPERS_CHECK_HOST:?set YUPPERS_CHECK_HOST to the build machine}"

ssh -o BatchMode=yes "$host" bash -s -- "$branch" "$what" <<'REMOTE'
set -euo pipefail
branch="$1"; what="$2"
export PATH="$HOME/.local/opt/node/bin:$HOME/.cargo/bin:$PATH" CARGO_INCREMENTAL=0
repo="$HOME/development/yuppers"
tag="$(printf %s "$branch" | tr -c 'a-zA-Z0-9' _ | cut -c1-40)_$$"
work="$HOME/development/yuppers-checks/$tag"
db="yuppers_check_$(printf %s "$tag" | tr 'A-Z' 'a-z' | cut -c1-40)"

set -a; . "$repo/.env"; set +a
owner_url="$MIGRATION_DATABASE_URL"
base="${owner_url%/*}"
app_url="${DATABASE_URL%/*}"

# Runs take turns: the end-to-end suites bind fixed ports, the target
# directory is shared, and the cleanup below drops every database that
# appeared during the run, which would be another run's if two overlapped.
exec 9>"$HOME/development/yuppers-checks.lock"
if ! flock -n 9; then echo "waiting for another check to finish..."; flock -w 10800 9; fi

# The test binaries each make a database of their own and leave it; every
# database that appears during the run is dropped with the run's own.
list_dbs() { docker exec yuppers-pg psql -U exchange -d postgres -Atc "select datname from pg_database where datname like 'yuppers_%'" </dev/null; }
cleanup() {
  cd "$repo"
  git worktree remove --force "$work" >/dev/null 2>&1 || true
  for d in $db $(comm -13 <(printf '%s\n' "$dbs_before" | sort) <(list_dbs | sort)); do
    psql_admin "DROP DATABASE IF EXISTS \"$d\" WITH (FORCE)" || true
  done
}
psql_admin() { docker exec yuppers-pg psql -q -U exchange -d postgres -c "$1" >/dev/null </dev/null; }
dbs_before="$(list_dbs)"
trap cleanup EXIT

cd "$repo"
git fetch -q origin "$branch" </dev/null
mkdir -p "$(dirname "$work")"
git worktree add -q --detach "$work" FETCH_HEAD
cd "$work"
echo "checking $branch at $(git rev-parse --short HEAD)"
cp "$repo/.env" .env
psql_admin "CREATE DATABASE $db OWNER exchange"
export MIGRATION_DATABASE_URL="$base/$db" DATABASE_URL="$app_url/$db"
sed -i -E "s|^MIGRATION_DATABASE_URL=.*|MIGRATION_DATABASE_URL=$MIGRATION_DATABASE_URL|; s|^DATABASE_URL=.*|DATABASE_URL=$DATABASE_URL|" .env
export CARGO_TARGET_DIR="$repo/backend/target"
# The end-to-end suites look for the binaries under the checkout; they are in
# the shared target directory instead.
export E2E_API_BIN="$CARGO_TARGET_DIR/debug/api" E2E_WORKER_BIN="$CARGO_TARGET_DIR/debug/worker" E2E_STAFF_BIN="$CARGO_TARGET_DIR/debug/staff"

status=0
step() { echo "== $1"; shift; if "$@" </dev/null; then echo "   ok"; else echo "   FAILED"; status=1; fi; }

if [ "$what" = all ] || [ "$what" = web ] || [ "$what" = e2e ]; then
  step "npm ci" npm ci --silent
fi
if [ "$what" != web ]; then
  step "migrate" bash -c 'cd backend && cargo run -q --bin migrate >/dev/null'
fi
if [ "$what" = all ] || [ "$what" = backend ]; then
  step "cargo fmt" bash -c 'cd backend && cargo fmt --check'
  step "cargo clippy" bash -c 'cd backend && cargo clippy -q --all-targets -- -D warnings'
  step "cargo test" bash -c 'cd backend && cargo test -q --no-fail-fast 2>&1 | tee /tmp/'"$tag"'.cargo.log | grep -E "^test result: FAILED|^---- " ; ! grep -q "test result: FAILED" /tmp/'"$tag"'.cargo.log'
fi
if [ "$what" = all ] || [ "$what" = web ]; then
  step "typecheck" npm run -s typecheck
  step "lint" bash -c 'npm run -s lint -w @yuppers/web && npm run -s lint -w @yuppers/mobile -- --max-warnings 0'
  # Longer timeouts than the five seconds the suites default to: the machine
  # is shared and busy, and a slow test is not a failed one.
  step "npm test" bash -c 'for w in @yuppers/shared @yuppers/web @yuppers/mobile; do npm run -s test -w $w -- --testTimeout=20000 || exit 1; done'

  step "build:web and budget" bash -c 'npm run -s build:web >/dev/null && npm run -s budget -w @yuppers/web'
fi
if [ "$what" = all ] || [ "$what" = e2e ]; then
  step "web e2e" npm run -s e2e
  step "mobile e2e" npm run -s e2e:mobile
fi
[ "$status" = 0 ] && echo "ALL CHECKS PASSED" || echo "SOME CHECKS FAILED"
exit "$status"
REMOTE
