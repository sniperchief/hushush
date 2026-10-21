# hushush backend: payroll-api + payout-wallet (+ the wallet CLI used once to
# create the wallet database). Zcash TESTNET ONLY. Build from the repo root:
#
#   docker build -f deploy/api.Dockerfile -t hushush-api .
#
# See docs/DEPLOY_RAILWAY.md.

FROM rust:1.99-slim-trixie AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config \
 && rm -rf /var/lib/apt/lists/*
# The librustzcash crates are heavy to compile: few parallel jobs keep the
# build within a small builder's memory.
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/build/target
WORKDIR /src
COPY payroll payroll
COPY payroll-api payroll-api
COPY wallet-proof wallet-proof
COPY payout-wallet payout-wallet
RUN cargo build --release --locked --manifest-path payroll-api/Cargo.toml \
 && cargo build --release --locked --manifest-path payout-wallet/Cargo.toml \
 && cargo build --release --locked --manifest-path wallet-proof/Cargo.toml --bin hushush-wallet-proof \
 && mkdir -p /out \
 && cp /build/target/release/hushush-payroll-api /build/target/release/hushush-payout-wallet /build/target/release/hushush-wallet-proof /out/

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates age tini \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/ /usr/local/bin/
COPY deploy/start.sh /app/start.sh
RUN chmod 0755 /app/start.sh
# The volume (wallet database, encrypted seed, payment intents) mounts at /data.
ENV HUSHUSH_DATA_DIR=/data/hushush \
    RUST_LOG=info,zcash_client_backend=warn \
    PORT=8080
EXPOSE 8080
ENTRYPOINT ["/usr/bin/tini", "--", "/app/start.sh"]
