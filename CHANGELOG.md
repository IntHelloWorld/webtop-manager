# Changelog

[简体中文](CHANGELOG.zh-CN.md)

All notable changes to Webtop Manager will be documented in this file. The
project follows [Semantic Versioning](https://semver.org/) once a version is
published.

## [Unreleased]

## [1.0.0] - 2026-09-03

### Added

- Initial stable release of the Linux x86_64 Tauri desktop application.
- Typed local Docker environment lifecycle management with ownership checks.
- Official image management, FRP publication controls, and portable templates.
- Durable controller operations, redacted progress output, and bilingual UI.
- Restart-resilient image pulls with persistent cancellation and UI reattachment.
- FRP remote-port race detection with serialized automatic retry.
- Transactional controller upgrades with state backup, schema migration,
  candidate health verification, and rollback.
- Docker-backed offline template, public TLS, and secret-leak release acceptance.

### Changed

- Removed routine FRP token rotation. Tokens are generated once, reused across
  reinstalls, fingerprint-checked, and recoverable only through a gated remote
  re-pairing flow when the protected local file is missing or invalid.
- Native managed frps setup now explicitly restarts its systemd service after
  updating configuration or recovery credentials.

### Security

- Hardened Unix Socket controller boundary and explicit Docker Socket warnings.
- Secret-file injection, canonical destructive-path validation, and conservative
  template import validation.
