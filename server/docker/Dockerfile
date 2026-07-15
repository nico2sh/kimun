# Kimün server image. Built by .github/workflows/build-server.yml, which
# downloads the release binaries compiled earlier in the same run and lays
# them out as dist/<TARGETARCH>/kimun-server — there is no Rust build here.
# Build context is the repository root:
#   docker buildx build -f server/docker/Dockerfile --platform linux/amd64,linux/arm64 .
# ubuntu:24.04 matches the CI build runners (ubuntu-latest / ubuntu-24.04-arm):
# the binary links their glibc (2.39), which debian bookworm (2.36) lacks.
FROM ubuntu:24.04

# ca-certificates: TLS roots for the embedding-model download and LLM API calls.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

ARG TARGETARCH
COPY dist/${TARGETARCH}/kimun-server /usr/local/bin/kimun-server
COPY server/docker/server.toml.default /usr/local/share/kimun/server.toml.default
COPY server/docker/entrypoint.sh /usr/local/bin/kimun-server-entrypoint
RUN chmod +x /usr/local/bin/kimun-server /usr/local/bin/kimun-server-entrypoint

# /data holds everything mutable: server.toml (seeded on first start), the
# SQLite vector store (/data/rag_sqlite), and the fastembed model cache
# (/data/.fastembed_cache — created relative to the workdir).
WORKDIR /data
VOLUME /data
EXPOSE 7573

ENTRYPOINT ["/usr/local/bin/kimun-server-entrypoint"]
