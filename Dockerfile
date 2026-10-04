FROM oven/bun:1 AS web-builder
WORKDIR /src
COPY . .
RUN bun install --frozen-lockfile && bun run build:web

FROM rust:1-bookworm AS rust-builder
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

FROM debian:bookworm-slim AS runtime
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
