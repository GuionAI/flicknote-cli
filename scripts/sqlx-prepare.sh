#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
META_DIR="$ROOT/.sqlx"
WORK_DIR="$ROOT/target/sqlx"
SQLITE_DB="$WORK_DIR/flicknote-sqlx.sqlite"
SQLITE_META="$WORK_DIR/sqlx-meta-sqlite"
PG_META="$WORK_DIR/sqlx-meta-postgres"
PG_CONTAINER="${SQLX_POSTGRES_CONTAINER:-flicknote-sqlx-pg}"
PG_PORT="${SQLX_POSTGRES_PORT:-55432}"
PG_URL="postgres://postgres:postgres@127.0.0.1:$PG_PORT/flicknote_sqlx"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

wait_for_postgres() {
  for _ in $(seq 1 60); do
    if docker exec "$PG_CONTAINER" pg_isready -U postgres -d flicknote_sqlx >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "postgres fixture did not become ready" >&2
  return 1
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
  require_cmd docker
  docker rm -f "$PG_CONTAINER" >/dev/null 2>&1 || true
  docker run \
    --name "$PG_CONTAINER" \
    -e POSTGRES_PASSWORD=postgres \
    -e POSTGRES_DB=flicknote_sqlx \
    -p "$PG_PORT:5432" \
    -d postgres:16-alpine >/dev/null
  trap 'docker rm -f "$PG_CONTAINER" >/dev/null 2>&1 || true' EXIT
  wait_for_postgres

  docker exec -i "$PG_CONTAINER" psql -U postgres -d flicknote_sqlx \
    <"$ROOT/scripts/sqlx-postgres-schema.sql" >/dev/null

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
