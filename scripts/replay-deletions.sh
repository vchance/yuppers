#!/bin/sh
# Applies a deletion log to a restored database, so that accounts deleted
# since the backup are deleted again (docs/operations.md, "Restoring").
#
#   scripts/replay-deletions.sh [-d APP_DATABASE_URL] FILE
#
# FILE is a log written by scripts/export-deletions.sh: the live database's,
# exported before restoring, or else the .deletions file of the newest
# backup there is, whichever backup was restored. Connects as the
# application role: -d, or else DATABASE_URL. Run it after restore.sh and
# migrate, before the api and the worker start on the copy, with
# CONTACT_DATA_KEY set to the key the backup was made under, which the
# binary needs (docs/operations.md, "Contact data key").
#
# Each account is deleted through the service's own deletion, by the
# replay-deletions binary (REPLAY_BIN, default
# backend/target/debug/replay-deletions; /usr/local/bin/replay-deletions in
# the image), so every rule runs again: sessions end, identifiers go,
# exchanges are left through the rules and the other party is told. An
# account already deleted, or one the copy never held, is skipped, so
# running it twice is harmless. It prints what it did for each account and
# exits non-zero if any is left undeleted. A line whose time the copy
# contradicts (before the account was created or last suspended there) is
# reported and left alone, and also leaves the exit status non-zero.
#
# Once every account in the log is deleted, it clears the mark restore.sh
# left, and the api and the worker can start on the copy.

set -eu

usage() {
    echo "usage: $0 [-d APP_DATABASE_URL] FILE" >&2
    exit 2
}

url="${DATABASE_URL:-}"
file=
while [ $# -gt 0 ]; do
    case "$1" in
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
    echo "$0: no database: give -d APP_DATABASE_URL or set DATABASE_URL" >&2
    exit 2
fi
if [ ! -r "$file" ]; then
    echo "$0: cannot read $file" >&2
    exit 2
fi

replay_bin="${REPLAY_BIN:-backend/target/debug/replay-deletions}"
DATABASE_URL="$url" NO_COLOR=1 exec "$replay_bin" "$file"
