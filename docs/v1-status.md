# v1 implementation status

This repository is an executable first milestone, not a claim that the full
product acceptance suite is complete.

## Implemented

- Independent Tauri 2 + React/TypeScript/Vite + Rust workspace.
- Docker-missing, permission, daemon and world-writable Socket diagnostics.
- Bundled controller-image build/load path and hardened persistent container configuration.
- `0600` Unix Socket `/v1` API and SQLite WAL state.
- Typed environment creation/start/stop/restart/delete commands.
- Reserved-field checks, non-privileged policy, app-label ownership and canonical delete containment.
- Password file injection without putting the password in Docker environment values.
- Snapshot worker preflight, special-file report, `tar.zst`, SHA-256 and atomic publication.
- Simplified Chinese and English shell, create flow, high-risk warnings and delete confirmation.
- Official LinuxServer Webtop image inventory, local-image detection, allowlisted Docker pulls,
  live layer progress/output and request cancellation.
- Persistent FRP server settings with the authentication token isolated in a `0600` file.
- Automatically generated FRP tokens, a copyable remote frps Docker setup guide,
  shared frpc start/restart/stop/status controls and authenticated connectivity tests.
- Explicit per-environment FRP publish/unpublish controls with automatic
  remote-port allocation, generated public links, copy support and frpc proxy
  refresh on lifecycle changes.
- Host-correct `/config` bind paths plus allowlisted opening of managed data directories.
- `.deb`/AppImage CI with checksums.
- frpc v0.70.1 amd64 manifest pinned as
  `sha256:e6483f2a916de67281597ba8fd03dc25d4f6fbd7ed0eafa042b2a5e4dcb5ee22`.
- Independent multi-layer templates created from stopped managed environments,
  with conservative preflight, a complete `/config` snapshot, metadata
  verification, lineage, source digest checks, missing-image detection and
  dependency-aware deletion.
- Versioned `.wtmpl` save/load transfer containing exactly a Docker save archive
  and `/config` snapshot, with strict static validation, hash/size checks,
  staging-tag rewriting and native UUID-only open/save bridging.
- SQLite-backed template operations with progress, typed results, UI task
  recovery, controller-restart retryable state and partial-file cleanup.

## Required before full v1 acceptance

- General-purpose cancellation and worker reattachment for non-template jobs.
- Safe-rebuild transaction with health verification and rollback.
- port-conflict retry after a concurrent external allocation race.
- Additional image lifecycle operations beyond the allowlisted official and
  app-owned template surfaces.
- Configurable XDG environment/snapshot roots.
- Controller backup/migration/rollback upgrade sequence.
- Full Docker offline template round-trip, frps/TLS, and secret-leak acceptance
  suites in CI (unit and UI coverage is present; daemon integration remains host-only).

These gaps are kept explicit so a development build cannot be mistaken for a
security-complete release.
