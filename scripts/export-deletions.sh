#!/bin/sh
# Exports the deletion log, the accounts deleted and when, to a file
# (docs/operations.md, "Restoring").
#
#   scripts/export-deletions.sh [--force] [-d DATABASE_URL] FILE
#
# Connects as the schema owner: -d, or else MIGRATION_DATABASE_URL. The file
# holds one line per deletion, the account's ID and the time in UTC, and
# nothing else; scripts/replay-deletions.sh applies it to a restored copy.
# scripts/backup.sh runs this for every backup, writing BACKUP.deletions
# beside it. Run it yourself against the live database before restoring a
# backup, while that database can still be reached.
#
# It refuses to replace a file that is already there unless given --force,
# and refuses a directory even then. The file is written under a fresh name
# beside FILE, readable by this user alone, and moved into place once whole.
# Set PG_BIN to the directory holding psql if the one on the PATH is older
# than the server.

set -eu

usage() {
    echo "usage: $0 [--force] [-d DATABASE_URL] FILE" >&2
    exit 2
}

url="${MIGRATION_DATABASE_URL:-}"
force=no
file=
while [ $# -gt 0 ]; do
    case "$1" in
        --force) force=yes ;;
        -d)
            [ $# -ge 2 ] || usage
            url="$2"
            shift
            ;;
        -h | --help) usage ;;
        -*) usage ;;
        *)
            [ -z "$file" ] || usage
            file="$1"
            ;;
    esac
    shift
done
[ -n "$file" ] || usage
if [ -z "$url" ]; then
    echo "$0: no database: give -d DATABASE_URL or set MIGRATION_DATABASE_URL" >&2
    exit 2
fi

bin="${PG_BIN:+$PG_BIN/}"

# The same rules as scripts/backup.sh: an existing file is never replaced by
# accident, a directory never, and a link is replaced itself, never followed.
if [ -d "$file" ]; then
    echo "$0: $file is a directory; give the name of the file to write" >&2
    exit 1
fi
if { [ -e "$file" ] || [ -L "$file" ]; } && [ "$force" != "yes" ]; then
    echo "$0: $file already exists; refusing to overwrite it. Give another name, or" >&2
    echo "pass --force to replace it." >&2
    exit 1
fi

umask 077
partial=$(mktemp "$(dirname -- "$file")/.$(basename -- "$file").partial.XXXXXX")
trap 'rm -f "$partial"' EXIT
trap 'exit 130' INT TERM
{
    echo "# Yuppers deletion log: account ID and deletion time (UTC), one per line;"
    echo "# for an account combined into another, that account's ID and EMAIL or PHONE."
    echo "# Apply to a restored database with scripts/replay-deletions.sh."
    "${bin}psql" --no-psqlrc --quiet --tuples-only --no-align --field-separator='	' \
        --set ON_ERROR_STOP=1 --dbname="$url" \
        --command="SELECT account_id,
                          to_char(deleted_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),
                          coalesce(merged_into::text, ''), coalesce(merged_by, '')
                   FROM deletion_log ORDER BY deleted_at, account_id"
} >"$partial"
count=$(grep -cv '^#' "$partial" || true)
if [ "$force" = "yes" ]; then
    mv -f -- "$partial" "$file"
else
    if ! ln -- "$partial" "$file"; then
        echo "$0: could not put the log in place as $file; anything there is left as it was" >&2
        exit 1
    fi
    rm -f -- "$partial"
fi
trap - EXIT INT TERM

echo "$file: $count deletions"
