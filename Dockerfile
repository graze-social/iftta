# syntax=docker/dockerfile:1.4
FROM rust:1.90-slim-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./

ARG FEATURES=embed
ARG TEMPLATES=./templates
ARG STATIC=./static

COPY src ./src
COPY benches ./benches
COPY migrations ./migrations
COPY static ./static
COPY ${TEMPLATES} ./templates
COPY ${STATIC} ./static

RUN cargo build --release --bin ifthisthenat --no-default-features --features ${FEATURES}

FROM gcr.io/distroless/cc-debian12

LABEL org.opencontainers.image.title="ifthisthenat"
LABEL org.opencontainers.image.description="If This Then AT - High-performance ATProtocol automation service"
LABEL org.opencontainers.image.licenses="MIT"
LABEL org.opencontainers.image.authors="Nick Gerakines <nick.gerakines@gmail.com>"
LABEL org.opencontainers.image.source="https://github.com/graze-social/ifthisthenat"
LABEL org.opencontainers.image.version="1.0.0"

WORKDIR /app

COPY --from=builder /app/target/release/ifthisthenat /app/ifthisthenat

COPY --from=builder /app/static ./static
COPY --from=builder /app/templates ./templates
COPY --from=builder /app/migrations ./migrations

ENV HTTP_PORT=8080 \
    HTTP_STATIC_PATH=/app/static \
    RUST_LOG=ifthisthenat=info,warning \
    RUST_BACKTRACE=1

# Expose default port
EXPOSE 8080

# Run the application
ENTRYPOINT ["/app/ifthisthenat"]
