# Stage 1: build the server (bundled DuckDB makes this the slow stage;
# CI caches cargo's registry via the workflow, not here).
FROM rust:1.92-slim AS build
# g++ for bundled DuckDB's C++ build (the CLI and server share one package,
# so the lib — DuckDB included — compiles even for the server binary).
RUN apt-get update \
    && apt-get install -y --no-install-recommends g++ \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
COPY config/ ./config/
RUN cargo build --release --bin server

# Stage 2: runtime. Litestream replicates the users DB to S3 so redeploys
# on a host with no persistent disk lose nothing.
FROM debian:bookworm-slim
ARG TARGETARCH
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
ADD https://github.com/benbjohnson/litestream/releases/download/v0.3.13/litestream-v0.3.13-linux-${TARGETARCH}.tar.gz /tmp/litestream.tar.gz
RUN tar -xzf /tmp/litestream.tar.gz -C /usr/local/bin && rm /tmp/litestream.tar.gz

WORKDIR /app
COPY --from=build /build/target/release/server ./server
COPY web/ ./web/
# The published dataset is baked at image build time; the deploy workflow
# regenerates it (ingest + publish) before `docker build`, and the daily
# schedule redeploys with fresh data.
COPY data/publish/ ./data/
COPY entrypoint.sh ./entrypoint.sh
RUN chmod +x ./entrypoint.sh

ENV ASFILED_SERVER_ADDR=0.0.0.0:8080 \
    ASFILED_STATIC_DIR=/app/web \
    ASFILED_DATA_DIR=/app/data \
    ASFILED_USERS_DB=/data/users.db

VOLUME /data
EXPOSE 8080
CMD ["./entrypoint.sh"]
