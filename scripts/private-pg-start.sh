#!/usr/bin/env bash
set -euo pipefail
# Called only inside the disposable test container, with a test-owned empty path.
export PATH=/usr/lib/postgresql/18/bin:$PATH
cluster="$1"
bootstrap="$2"
initdb -D "$cluster" -A trust --no-instructions
cat >> "$cluster/postgresql.conf" <<'CONF'
shared_preload_libraries = 'pgroonga_wal_resource_manager,pgroonga_crash_safer'
pgroonga.enable_wal_resource_manager = on
pgroonga.enable_crash_safe = on
wal_level = logical
listen_addresses = '*'
CONF
printf '%s\n' 'host all all 0.0.0.0/0 trust' >> "$cluster/pg_hba.conf"
pg_ctl -D "$cluster" -l "$cluster/server.log" start
createdb -h 127.0.0.1 -p 5432 -U postgres supabase
psql -X -h 127.0.0.1 -p 5432 -U postgres -d supabase -v ON_ERROR_STOP=1 -f "$bootstrap"
