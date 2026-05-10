#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
META_DIR="$ROOT/.sqlx"
WORK_DIR="$ROOT/target/sqlx"
SQLITE_DB="$WORK_DIR/flicknote-sqlx.sqlite"
SQLITE_META="$WORK_DIR/sqlx-meta-sqlite"
PG_META="$WORK_DIR/sqlx-meta-postgres"
PG_URL="${SQLX_POSTGRES_DATABASE_URL:-postgres://supabase_admin:dev-password@localhost:30432/supabase?search_path=public}"

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

  rm -rf "$SQLITE_META"
  mkdir -p "$SQLITE_META"
  cp "$META_DIR"/*.json "$SQLITE_META"/
}

prepare_postgres() {
  rm -rf "$META_DIR"
  cargo sqlx prepare --workspace -D "$PG_URL" -- \
    -p flicknote-core \
    --no-default-features \
    --features storage-pgwire \
    --all-targets

  rm -rf "$PG_META"
  mkdir -p "$PG_META"
  cp "$META_DIR"/*.json "$PG_META"/
}

mkdir -p "$WORK_DIR"
prepare_sqlite
prepare_postgres

rm -rf "$META_DIR"
mkdir -p "$META_DIR"
cp "$SQLITE_META"/*.json "$META_DIR"/
cp "$PG_META"/*.json "$META_DIR"/
