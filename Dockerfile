# Multi-stage build for oa-gateway. Schema is fetched at image build so the
# runtime image is self-contained; override with a bind-mount if you bring your
# own message set.
#
#   docker build -t oa-gateway .
#   docker compose -f compose/gateway.yml up --build

FROM rust:1.85.0-bookworm AS builder

WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY scripts ./scripts

# Fetch before compiling so a schema download failure fails the build early,
# rather than after a long cargo run.
RUN ./scripts/fetch-uci-schema.sh

RUN cargo build --release -p oa-gateway --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /app --shell /usr/sbin/nologin oag

WORKDIR /app

COPY --from=builder /src/target/release/oa-gateway /usr/local/bin/oa-gateway
COPY --from=builder /src/schema/uci /app/schema/uci

USER oag
EXPOSE 9000
ENTRYPOINT ["oa-gateway"]
# Config path is required. compose/gateway.yml mounts one and passes it as argv.
