#!/usr/bin/env bash
set -euo pipefail

TARGET="x86_64-unknown-linux-musl"
BINARY="target/${TARGET}/release/flicknote"
CONTEXT="--context guion-tunnel"
NAMESPACE="-n apps-dev"
LABEL="app.kubernetes.io/name=temenos"
DEST="/usr/local/bin/note"

echo "==> Linting..."
cargo clippy -p flicknote-cli -- -D warnings
cargo fmt

echo "==> Building flicknote for ${TARGET}..."
cargo zigbuild --release --target "$TARGET" -p flicknote-cli

echo "==> Copying to cluster..."
POD=$(kubectl get pods $CONTEXT $NAMESPACE -l "$LABEL" -o jsonpath='{.items[0].metadata.name}')
kubectl cp $CONTEXT "$BINARY" "${NAMESPACE#-n }/${POD}:${DEST}"

echo "==> Done."
