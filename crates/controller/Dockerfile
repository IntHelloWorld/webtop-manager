# syntax=docker/dockerfile:1.7
FROM --platform=$BUILDPLATFORM rust:1.98.0-alpine@sha256:a10e64dd139b7387337c7fbe8aca31b959b57b2fd4c8ae20a02cf1d6ea424dce AS builder
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
