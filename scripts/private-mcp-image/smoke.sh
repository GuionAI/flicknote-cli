#!/bin/sh
set -eu
expected="$1"
test "$(id -u):$(id -g)" = 65532:65532
test -s "$SSL_CERT_FILE"
test -r "$SSL_CERT_FILE"
test -s /usr/share/doc/private-mcp/LICENSE
test "$(/usr/local/bin/flicknote --version)" = "flicknote ${expected#v}"
/usr/local/bin/flicknote private-mcp --help | grep -q -- --allowed-host
FLICKNOTE_PRIVATE_DATABASE_URL='postgres://flicknote_mcp@127.0.0.1:1/postgres' \
 /usr/local/bin/flicknote private-mcp --listen 127.0.0.1:8080 \
 --resource http://127.0.0.1:8080/mcp --issuer http://127.0.0.1:1 \
 --verifier http://127.0.0.1:1/oauth/mcp/verify \
 --allowed-host 127.0.0.1:8080 --allowed-origin http://127.0.0.1:8080 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true' EXIT
i=0
until grep -q ':1F90 .* 0A ' /proc/net/tcp; do
 kill -0 "$pid"
 i=$((i + 1)); test "$i" -lt 50
 sleep 0.1
done
test ! -e "$HOME"
