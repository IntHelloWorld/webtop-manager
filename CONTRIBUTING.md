# Contributing

[简体中文](CONTRIBUTING.zh-CN.md)

Webtop Manager is an alpha-stage, security-sensitive desktop application. Small,
focused changes with tests and an explicit security rationale are easiest to
review.

## Development setup

The supported development environment is Ubuntu 24.04 on Linux x86_64 with a
local Docker Engine. Follow [docs/development.md](docs/development.md), then run:

```bash
./scripts/doctor.sh
./scripts/check.sh
cargo check --package webtop-manager
```

Controller or worker changes also require rebuilding and validating the bundled
OCI archive:

```bash
./scripts/package-controller.sh
zstd --test --quiet src-tauri/assets/controller-image.tar.zst
```

The generated archive is intentionally ignored by Git and is rebuilt by the
release workflow.

## Pull requests

- Keep the WebView API typed and allowlisted; do not add generic shell, Docker,
  path deletion, or URL-opening commands.
- Preserve the application-owned label and canonical-path checks around every
  destructive operation.
- Never place passwords, tokens, private host paths, or command output in API
  values, SQLite records, logs, fixtures, or frontend events.
- Keep Internet publication disabled by default and visibly risk-confirmed.
- Add or update Rust and frontend tests for observable behavior.
- Update security, architecture, and status documentation when a boundary or
  acceptance gap changes.
- Do not commit `.wtmpl`, `.env*`, database, certificate, package, controller
  archive, or application-data files.

Use the pull request template and make sure every applicable check is complete.
Security vulnerabilities must be reported privately as described in
[SECURITY.md](SECURITY.md), not through a pull request or public issue.
