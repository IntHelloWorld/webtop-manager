# syntax=docker/dockerfile:1.7
FROM --platform=$BUILDPLATFORM rust:1.99.1-alpine@sha256:557a94c82b485a14b569120e00867267108c734efe33f10cc31ca0cd5dd1eca9 AS builder
RUN apk add --no-cache musl-dev pkgconfig
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates/contracts crates/contracts
COPY crates/controller crates/controller
COPY crates/worker crates/worker
COPY src-tauri src-tauri
RUN cargo build --locked --release --package webtop-controller --package webtop-worker

FROM scratch
COPY --from=builder /src/target/release/webtop-controller /usr/local/bin/webtop-controller
COPY --from=builder /src/target/release/webtop-worker /usr/local/bin/webtop-worker
ENTRYPOINT ["/usr/local/bin/webtop-controller"]
