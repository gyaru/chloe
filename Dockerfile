FROM rust:1.87 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    libpq-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

COPY Cargo.toml Cargo.lock ./

RUN mkdir src && echo "fn main() {println!(\"Dummy main for caching dependencies\")}" > src/main.rs
RUN cargo build --release

COPY src ./src

RUN cargo build --release

FROM debian:bullseye-slim AS final

RUN apt-get update && apt-get install -y --no-install-recommends \
    libpq5 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid 1000 appuser && \
    useradd --uid 1000 --gid 1000 --shell /bin/bash --create-home appuser

WORKDIR /app

COPY --from=builder /usr/src/app/target/release/chloe .

RUN chmod +x ./chloe

USER appuser

ENTRYPOINT ["./chloe"] 