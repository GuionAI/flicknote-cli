#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
META_DIR="$ROOT/.sqlx"
WORK_DIR="$ROOT/target/sqlx"
SQLITE_DB="$WORK_DIR/flicknote-sqlx.sqlite"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

prepare_sqlite() {
  require_cmd sqlite3
  rm -f "$SQLITE_DB"
  sqlite3 "$SQLITE_DB" <"$ROOT/scripts/sqlx-sqlite-schema.sql"

  rm -rf "$META_DIR"
  cargo sqlx prepare --workspace -D "sqlite://$SQLITE_DB" -- \
    -p flicknote-core \
    --no-default-features \
    --features powersync \
    --all-targets

}

mkdir -p "$WORK_DIR"
prepare_sqlite
