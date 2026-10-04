# Base images are pinned by digest. Floating tags (`rust:1`, `oven/bun:1`)
# must be resolved through the registry at build time, and a stalled manifest
# fetch wedges the whole deployment with no useful error.
FROM oven/bun:1@sha256:9114c058aeae42162ee16dd5084b95fe9473970bb6bcb5b232ab1630f0546895 AS web-builder
WORKDIR /src
COPY . .
RUN bun install --frozen-lockfile && bun run build:web

# rust:1 tracks the stable channel pinned in rust-toolchain.toml.
FROM rust:1-bookworm@sha256:59037199c44290f2befcdd58dcc540164763fc296950255aaefeef096a1866b0 AS rust-builder
WORKDIR /src
COPY . .
COPY --from=web-builder /src/web/dist ./web/dist
# prost needs protoc; the schema imports google/protobuf/struct.proto, so the
# well-known .proto files (libprotobuf-dev) are required too, not just the
# compiler; ring/aws-lc-sys need cmake + C toolchain
RUN apt-get update \
    && apt-get install --no-install-recommends -y protobuf-compiler libprotobuf-dev cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN cargo build --locked --release -p conex-host -p conex-agent

FROM debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251 AS runtime
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /nonexistent --shell /usr/sbin/nologin conex
COPY --from=rust-builder /src/target/release/conex-host /usr/local/bin/conex-host
COPY --from=rust-builder /src/target/release/conex-agent /usr/local/bin/conex-agent
COPY --from=web-builder /src/web/dist /opt/conex/web
COPY docker/host.toml /etc/conex/host.toml
# Synthetic demo tree backing the read-only endpoints in host.toml. This is
# the public demo deployment; a production stack mounts real agent data
# instead (see deployment/docker-compose.yml).
COPY docker/demo /opt/conex/demo
RUN mkdir -p /etc/conex /var/lib/conex/content /var/lib/conex/session /var/lib/conex/operation \
    && chown -R conex:conex /var/lib/conex /etc/conex/host.toml \
    && chmod -R a+rX /opt/conex/demo
USER conex
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/conex-host"]
CMD ["/etc/conex/host.toml"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD curl --fail --silent --show-error http://127.0.0.1:8787/ >/dev/null || exit 1
