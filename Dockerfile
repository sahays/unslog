# syntax=docker/dockerfile:1.7
# Multi-stage build: cargo + npm (tailwind) → slim runtime.

# ── Stage 1: build ─────────────────────────────────────────────
FROM rust:1.82-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config libssl-dev nodejs npm ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache npm deps (tailwind cli) — build.rs shells out to npx during cargo build.
COPY package.json package-lock.json ./
RUN npm ci --silent

# Cache cargo deps.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main(){}" > src/main.rs \
 && cargo build --release --bin unslog \
 && rm -rf src target/release/deps/unslog*

# Real source + assets.
COPY build.rs ./
COPY src ./src
COPY templates ./templates
COPY static ./static
COPY migrations ./migrations
COPY prompts ./prompts
COPY askama.toml ./
# sqlx compile-time macros (`query!`, `query_as!`, ...) read prepared
# query metadata from `.sqlx/` when SQLX_OFFLINE=true — no live DB at build.
COPY .sqlx ./.sqlx

ENV SQLX_OFFLINE=true
RUN cargo build --release --bin unslog

# ── Stage 2: runtime ───────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates poppler-utils curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 10001 --home /app --create-home unslog

WORKDIR /app

COPY --from=builder /build/target/release/unslog /usr/local/bin/unslog
COPY --from=builder /build/static /app/static
COPY --from=builder /build/templates /app/templates
COPY --from=builder /build/migrations /app/migrations
COPY --from=builder /build/prompts /app/prompts

RUN mkdir -p /app/data && chown -R unslog:unslog /app

USER unslog
ENV HOST=0.0.0.0 \
    PORT=3000 \
    DATA_DIR=/app/data \
    LOG_DIR=/app/data/logs \
    RUST_LOG=info,unslog=info

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD curl -fsS http://127.0.0.1:3000/health || exit 1

ENTRYPOINT ["/usr/local/bin/unslog"]
