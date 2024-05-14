FROM debian:bookworm-slim

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        build-essential \
        libclang-dev \
        libssl-dev \
        pkg-config \
        openssl \
        protobuf-compiler \
        ca-certificates \
        curl

COPY src/ src/
COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY rust-toolchain.toml rust-toolchain.toml

COPY state-reconstruct-fetcher/ state-reconstruct-fetcher/
COPY state-reconstruct-storage/ state-reconstruct-storage/

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
ENV PATH="/root/.cargo/bin:${PATH}"

RUN cargo build --release
ENV PATH="/target/release:${PATH}"

COPY abi/ abi/
COPY InitialState.csv InitialState.csv

CMD ["state-reconstruct", "reconstruct", "l1", "--http-url", "https://eth.llamarpc.com"]
