# Kimün server image. Built by .github/workflows/build-server.yml, which
# downloads the release binaries compiled earlier in the same run and lays
# them out as dist/<TARGETARCH>/kimun-server — there is no Rust build here.
# Build context is the repository root:
#   docker buildx build -f server/docker/Dockerfile --platform linux/amd64,linux/arm64 .

# ca-certificates staged on the BUILD platform: the cert bundle is
# arch-independent, and running apt under qemu emulation for the arm64 layer
# is both slow and flaky (this exact apt-get failed in CI under emulation).
# With the bundle COPY'd in, the final stage has no RUN at all, so the
# non-native platform builds without any emulation.
FROM --platform=$BUILDPLATFORM ubuntu:24.04 AS certs
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# ubuntu:24.04 matches the CI build runners (ubuntu-latest / ubuntu-24.04-arm):
# the binary links their glibc (2.39), which debian bookworm (2.36) lacks.
FROM ubuntu:24.04

# Links the GHCR package to the repository (repo page shows the package, the
# package page shows the README).
LABEL org.opencontainers.image.source="https://github.com/nico2sh/kimun" \
      org.opencontainers.image.description="Kimün RAG server — semantic search and Ask for Kimün vaults" \
      org.opencontainers.image.licenses="MIT"

# TLS roots for the embedding-model download and LLM API calls.
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt

ARG TARGETARCH
COPY --chmod=755 dist/${TARGETARCH}/kimun-server /usr/local/bin/kimun-server
COPY server/docker/server.toml.default /usr/local/share/kimun/server.toml.default
COPY --chmod=755 server/docker/entrypoint.sh /usr/local/bin/kimun-server-entrypoint

# /data holds everything mutable: server.toml (seeded on first start), the
# SQLite vector store (/data/rag_sqlite), and the fastembed model cache
# (/data/.fastembed_cache — created relative to the workdir).
WORKDIR /data
VOLUME /data
EXPOSE 7573

ENTRYPOINT ["/usr/local/bin/kimun-server-entrypoint"]
