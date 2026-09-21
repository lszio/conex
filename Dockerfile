FROM oven/bun:1 AS web-builder
WORKDIR /src
COPY . .
RUN bun install --frozen-lockfile && bun run build:web

FROM rust:1.85-bookworm AS rust-builder
WORKDIR /src
COPY . .
COPY --from=web-builder /src/web/dist ./web/dist
RUN cargo build --locked --release -p conex-host -p conex-agent

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /nonexistent --shell /usr/sbin/nologin conex
COPY --from=rust-builder /src/target/release/conex-host /usr/local/bin/conex-host
COPY --from=rust-builder /src/target/release/conex-agent /usr/local/bin/conex-agent
COPY --from=web-builder /src/web/dist /opt/conex/web
RUN mkdir -p /etc/conex /var/lib/conex \
    && chown -R conex:conex /var/lib/conex
USER conex
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/conex-host"]
CMD ["/etc/conex/host.toml"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD curl --fail --silent --show-error http://127.0.0.1:8787/ >/dev/null || exit 1
