# Version 1.0 implementation status

[简体中文](v1-status.zh-CN.md)

Version 1.0 satisfies the release acceptance scope defined for this repository.
The product boundary remains intentionally narrow: Linux x86_64, a local Docker
Engine, and newly created Webtop Manager environments only.

## Accepted for 1.0

- Independent Tauri 2 + React/TypeScript/Vite + Rust workspace.
- Docker-missing, permission, daemon, and world-writable Socket diagnostics.
- Bundled controller-image build/load path and hardened persistent container.
- Mode-`0600` Unix Socket `/v1` API, versioned SQLite schema, and WAL state.
- Typed environment creation/start/stop/restart/delete operations.
- Reserved-field checks, non-privileged policy, ownership labels, and canonical
  deletion containment.
- Password file injection without putting passwords in Docker environment
  values, SQLite, logs, or frontend events.
- Official LinuxServer Webtop inventory, local-image detection, allowlisted
  pulls, live progress, explicit cancellation, controller-restart resumption,
  and desktop reattachment.
- Persistent FRP settings with the token isolated in a mode-`0600` file.
- One-time generated FRP tokens, fingerprint-backed missing-token detection,
  gated remote re-pairing, server setup guides, shared frpc lifecycle controls,
  authenticated connectivity tests, and explicit per-environment publication.
- Serialized remote-port allocation with detection and retry when an external
  FRP client wins a concurrent allocation race.
- Host-correct `/config` bind paths and allowlisted managed-directory opening.
- Independent multi-layer templates with a complete `/config` snapshot,
  conservative preflight, metadata and digest checks, lineage, and
  dependency-aware deletion.
- Versioned `.wtmpl` export/import with fixed payloads, offline Docker load,
  traversal protection, hash/size validation, and UUID-only native staging.
- Persistent template operations with bounded redacted output, cooperative
  cancellation, restart-safe terminal state, and partial-artifact cleanup.
- Controller upgrades that import before interruption, back up protected state,
  reject newer schemas, run a candidate against migrated state, verify health,
  atomically switch container names, and restore the previous state/controller
  on failure.
- Docker-backed release acceptance for offline template round-trips, concurrent
  FRP port conflicts, local and public HTTPS/TLS, and secret-leak checks across
  Docker Inspect, API responses, SQLite, manifests, and controller logs.
- Linux x86_64 `.deb` and AppImage release packages with SHA-256 checksums.
- Complete Simplified Chinese and English desktop UI, README, and project documentation.

## Explicitly outside the 1.0 scope

The following are product non-goals for 1.0 and do not block release:

- Safe rebuild transactions for changing immutable environment configuration.
- General-purpose image lifecycle operations beyond the allowlisted official
  catalog and application-owned template surfaces.
- User-configurable XDG environment and snapshot roots.

Existing Compose projects and containers also remain outside the ownership
boundary: Webtop Manager never imports, adopts, rebuilds, or deletes them.

## Release gates

Every 1.0 release must pass:

```bash
./scripts/check.sh
cargo check --package webtop-manager --locked
./scripts/check-release-version.sh v1.0.0
./scripts/package-controller.sh
zstd --test --quiet src-tauri/assets/controller-image.tar.zst
./scripts/test-packaged-controller.sh
./scripts/test-docker-acceptance.sh
```

The Docker-backed suite runs on `main`, by manual CI dispatch, and during the
release workflow before package creation. It uses isolated state, sockets, FRP
containers, and application-owned test resources.
