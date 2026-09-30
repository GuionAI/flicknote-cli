#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="${FLICKNOTE_TEST_PG_IMAGE:-ghcr.io/guionai/cloudnative-supabase-postgres-pgroonga@sha256:d555bf68fad60626664e22cf90390fc8936b42092d504f7f02a592ec70b92626}"
engine="${CONTAINER_TOOL:-podman}"
mode="${1:-check}"
[[ "$mode" == check || "$mode" == --provision-only ]] || { echo 'Usage: test-private-pg.sh [--provision-only]' >&2; exit 2; }
work="$(mktemp -d -t flicknote-pg.XXXXXXXX)"
container_name="flicknote-cli-pg-test-$(date +%s)-$$"
owned_source=""
owned_lock=""
cleanup() {
  if [[ "${FLICKNOTE_TEST_PG_IN_CONTAINER:-}" == 1 ]]; then
    pg_ctl -D "$work/pg" -m immediate stop >/dev/null 2>&1 || true
  else
    "$engine" rm -f "$container_name" >/dev/null 2>&1 || true
  fi
  rm -rf "$work"
  if [[ -n "$owned_source" ]]; then rm -rf "$owned_source"; fi
  if [[ -n "$owned_lock" ]]; then rmdir "$owned_lock"; fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ -n "${FLICKNOTE_TEST_FB_CHECKOUT:-}" ]]; then
  source_root="$(cd "$FLICKNOTE_TEST_FB_CHECKOUT" && pwd)"
  revision="$(git -C "$source_root" rev-parse "${FLICKNOTE_TEST_FB_REVISION:-HEAD}^{commit}")"
  dirty="$(git --no-optional-locks -C "$source_root" status --porcelain)"
  input_dirty="$([[ -n "$dirty" ]] && echo true || echo false)"
  if [[ -n "${FLICKNOTE_TEST_FB_REVISION:-}" ]]; then dirty=""; fi
  echo "FB_SOURCE=$source_root FB_COMMIT=$revision FB_INPUT_DIRTY=$input_dirty FB_DIRTY=$([[ -n "$dirty" ]] && echo true || echo false)"
  if [[ -n "${FLICKNOTE_TEST_FB_REVISION:-}" ]]; then
    git -C "$source_root" archive "$revision" | tar -x -C "$work"
  else
    cp -R "$source_root/tanka" "$work/"
  fi
else
  # og's reference destination is fixed. Refuse existing/user-owned state;
  # this run owns the fresh clone and removes it on exit.
  source_root="$HOME/code/references/forgejo.localhost_17480/guionai/flick-backend"
  mkdir -p "$(dirname "$source_root")"
  mkdir "$source_root.flicknote-pg-lock" || { echo "Another mainline run owns the source lock" >&2; exit 1; }
  owned_lock="$source_root.flicknote-pg-lock"
  if [[ -e "$source_root" ]]; then
    echo "Source reference already exists: $source_root; use FLICKNOTE_TEST_FB_CHECKOUT or move it aside before a mainline run" >&2
    exit 1
  fi
  owned_source="$source_root"
  og clone --reference http://forgejo.localhost:17480/GuionAI/flick-backend.git --json > "$work/clone.json"
  source_root="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["clone"]["path"])' < "$work/clone.json")"
  revision="$(git -C "$source_root" rev-parse "${FLICKNOTE_TEST_FB_REVISION:-refs/remotes/origin/main}^{commit}")"
  echo "FB_SOURCE=mainline FB_COMMIT=$revision FB_DIRTY=false"
  git -C "$source_root" archive "$revision" | tar -x -C "$work"
fi
migrations="$work/tanka/charts/db-init/db/migrations"
shopt -s nullglob
files=("$migrations"/*.sql)
[[ ${#files[@]} -gt 0 ]] || { echo 'No fb migrations found' >&2; exit 1; }
for file in "${files[@]}"; do basename "$file" | cut -d_ -f1; done | sort > "$work/expected"

if [[ "${FLICKNOTE_TEST_PG_IN_CONTAINER:-}" == 1 ]]; then
  export PATH=/usr/lib/postgresql/18/bin:$PATH
  bash "$repo_root/scripts/private-pg-start.sh" "$work/pg" "$repo_root/flicknote-sync/tests/fixtures/private_pg_bootstrap.sql"
  export FLICKNOTE_TEST_PG_PORT=5432
  sql() { psql -X -h 127.0.0.1 -p "$FLICKNOTE_TEST_PG_PORT" -U postgres -d supabase -v ON_ERROR_STOP=1 "$@"; }
  echo "PG_IMAGE=$image (Dagger test container)"
else
  echo "PG_CONTAINER=$container_name"
  "$engine" run -d --name "$container_name" -p 127.0.0.1::5432 \
    -v "$repo_root/flicknote-sync/tests/fixtures/private_pg_bootstrap.sql:/bootstrap.sql:ro" \
    -v "$repo_root/scripts/private-pg-start.sh:/start.sh:ro" \
    --entrypoint bash "$image" -c 'bash /start.sh /tmp/fnpg /bootstrap.sql && touch /tmp/ready && exec sleep infinity' >/dev/null
  ready=false
  for ((i=0;i<60;i++)); do
    if "$engine" exec "$container_name" test -f /tmp/ready; then ready=true; break; fi
    if [[ "$("$engine" inspect --format '{{.State.Running}}' "$container_name")" != true ]]; then break; fi
    sleep 1
  done
  if [[ "$ready" != true ]]; then "$engine" logs "$container_name"; exit 1; fi
  echo "PG_IMAGE=$image PG_IMAGE_ID=$("$engine" inspect --format '{{.Image}}' "$container_name")"
  mapped="$("$engine" port "$container_name" 5432/tcp)"
  export FLICKNOTE_TEST_PG_PORT="${mapped##*:}"
  sql() { "$engine" exec -i "$container_name" psql -X -h 127.0.0.1 -p 5432 -U postgres -d supabase -v ON_ERROR_STOP=1 "$@"; }
fi
# Explicit URL and env file prevent dbmate from reading a live .env/database.
dbmate --url "postgres://postgres@127.0.0.1:$FLICKNOTE_TEST_PG_PORT/supabase?sslmode=disable" \
  --env-file /dev/null --migrations-dir "$migrations" --no-dump-schema migrate
sql -Atc 'SELECT version FROM schema_migrations ORDER BY version' > "$work/applied"
diff -u "$work/expected" "$work/applied"
echo "FB_MIGRATIONS_APPLIED=${#files[@]}"
sql < "$work/tanka/charts/db-init/db/opt-in/private-mcp.sql"
sql <<'SQL'
ALTER ROLE flicknote_mcp LOGIN;
INSERT INTO auth.users(id) VALUES ('11111111-1111-4111-8111-111111111111'), ('22222222-2222-4222-8222-222222222222');
SQL
[[ "$mode" != --provision-only ]] || exit 0
cd "$repo_root"
cargo test -p flicknote-sync --lib pg::tests::cancelled_identity_setup_discards_backend_before_pool_reuse -- --ignored --nocapture
cargo test -p flicknote-sync --test private_pg -- --ignored --nocapture --test-threads=1
