#!/usr/bin/env bash
set -euo pipefail

# Own all database state: a fresh container, in-container cluster and fixture.
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="${FLICKNOTE_TEST_PG_IMAGE:-localhost/cnsupa-postgres-pgroonga:chatgpt-mcp-pg}"
container_name="flicknote-cli-pg-test-$(date +%s)-$$"
container_id="$(podman run -d --rm --name "$container_name" -p 127.0.0.1::5432 \
  -v "$repo_root/flicknote-sync/tests/fixtures/private_pg.sql:/fixture.sql:ro" \
  --entrypoint bash "$image" -c '
    set -e
    export PATH=/usr/lib/postgresql/18/bin:$PATH
    initdb -D /tmp/fnpg -A trust --no-instructions >/tmp/init.log
    cat >> /tmp/fnpg/postgresql.conf <<CONF
shared_preload_libraries = '\''pgroonga_wal_resource_manager,pgroonga_crash_safer'\''
pgroonga.enable_wal_resource_manager = on
pgroonga.enable_crash_safe = on
listen_addresses = '\''*'\''
CONF
    printf "%s\n" "host all all 0.0.0.0/0 trust" >> /tmp/fnpg/pg_hba.conf
    pg_ctl -D /tmp/fnpg -l /tmp/fnpg.log start
    psql -h 127.0.0.1 -U postgres -d postgres -v ON_ERROR_STOP=1 -f /fixture.sql >/tmp/fixture.log
    sleep infinity
  ')"
cleanup() { podman rm -f "$container_id" >/dev/null 2>&1 || true; }
trap cleanup EXIT
for _ in $(seq 1 60); do
  if podman exec "$container_id" test -f /tmp/fixture.log && podman exec "$container_id" psql -h 127.0.0.1 -U flicknote_test -d postgres -Atc 'SELECT extversion FROM pg_extension WHERE extname = '\''pgroonga'\''' 2>/dev/null | grep -q '^4\.'; then break; fi
  if ! podman container exists "$container_id"; then podman logs "$container_id"; exit 1; fi
  sleep 1
done
mapped="$(podman port "$container_id" 5432/tcp)"
export FLICKNOTE_TEST_PG_PORT="${mapped##*:}"
cd "$repo_root"
cargo test -p flicknote-sync --lib pg::tests::cancelled_identity_setup_discards_backend_before_pool_reuse -- --ignored --nocapture
cargo test -p flicknote-sync --test private_pg -- --ignored --nocapture --test-threads=1
