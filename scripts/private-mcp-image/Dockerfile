FROM debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY flicknote /usr/local/bin/flicknote
COPY notices/ /usr/share/doc/private-mcp/
ARG RELEASE_TAG
ARG ARTIFACT_SHA256
ARG ARTIFACT_URL
LABEL org.opencontainers.image.source="https://github.com/GuionAI/flicknote-cli" \
      org.opencontainers.image.version="${RELEASE_TAG}" \
      org.opencontainers.image.url="${ARTIFACT_URL}" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.base.name="debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251" \
      app.flicknote.artifact.sha256="${ARTIFACT_SHA256}" \
      app.flicknote.target="x86_64-unknown-linux-musl"
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt HOME=/tmp/private-mcp
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/flicknote"]
CMD ["private-mcp", "--help"]
