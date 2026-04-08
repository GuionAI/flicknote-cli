#!/usr/bin/env bash
set -euo pipefail

TARGET="x86_64-unknown-linux-musl"
BINARY="target/${TARGET}/release/flicknote"

echo "==> Linting..."
cargo clippy -p flicknote-cli -- -D warnings
cargo fmt

echo "==> Building flicknote for ${TARGET}..."
cargo zigbuild --release --target "$TARGET" -p flicknote-cli

echo "==> Copying to cluster..."
kubectl cp --context guion-tunnel \
  "$BINARY" \
  apps-dev/temenos-6fbdff8fd5-rdqb9:/usr/local/bin/note

echo "==> Done."
