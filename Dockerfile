FROM rustlang/rust:nightly-bookworm-slim

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        build-essential \
        libclang-dev \
        libssl-dev \
        pkg-config

COPY ./src ./src
COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock

RUN cargo build --release

FROM debian:bookworm-slim

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        openssl

COPY --from=0 ./target/release/state-reconstruct /state-reconstruct
COPY IZkSync.json IZkSync.json
COPY InitialState.csv InitialState.csv

CMD ["/state-reconstruct", "reconstruct", "l1", "--http-url", "https://eth.llamarpc.com"]
