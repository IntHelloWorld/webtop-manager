# Webtop Manager

Webtop Manager is a bilingual Linux desktop application for creating and
operating private [LinuxServer Webtop](https://docs.linuxserver.io/images/docker-webtop/)
environments on a local Docker Engine. It uses Tauri 2, React/TypeScript, and a
persistent Rust controller with a typed Unix Socket API.

> **Project status: alpha preview.** The current release is intended for
> evaluation and personal testing. It is not yet a security-complete stable v1;
> review the [known acceptance gaps](docs/v1-status.md#required-before-full-v1-acceptance)
> before installing it on an important host.

## Supported scope

- Linux x86_64; Ubuntu 24.04 is the currently tested distribution.
- A local Docker Engine that is installed and administered separately.
- New Webtop Manager environments only. Existing Compose projects and
  containers are never imported, adopted, modified, or deleted.
- `.deb` and AppImage packages produced by GitHub Actions.
- No automatic updater in the alpha release.

## Features

- Create, start, stop, restart, and safely delete application-owned Webtops.
- Browse and pull an allowlisted catalog of official LinuxServer images.
- Configure a shared frpc client and explicitly publish individual environments.
- Create portable multi-layer templates with a separate complete `/config`
  snapshot, then import or export versioned `.wtmpl` packages.
- Keep long template operations in the controller so they survive UI closure.
- Show bounded, redacted progress without exposing arbitrary command output.
- Use Simplified Chinese or English throughout the desktop interface.

## Security warning

Access to `/var/run/docker.sock` is effectively root-equivalent. Webtop Manager
keeps that access behind a persistent controller, a mode-`0600` Unix Socket,
typed operations, ownership labels, and canonical path checks, but installing
the application still grants it control over the local Docker daemon.

Internet publication is disabled by default and requires explicit confirmation.
The alpha release provides personal-use TCP forwarding, not a complete reverse
proxy or enterprise authentication layer. Read the [security model](docs/security.md)
and [FRP setup guide](docs/frps-setup.md) before enabling publication.

Portable `.wtmpl` files and snapshots are not encrypted. They may contain SSH
keys, browser profiles, cloud credentials, and other secrets copied from
`/config`. Never attach them to issues or commit them to a repository.

## Install a pre-release

1. Install Docker Engine using your distribution or administrator's process.
   The application never installs Docker, changes Docker groups, or modifies
   Socket permissions.
2. Download `SHA256SUMS` and either the `.deb` or AppImage from the repository's
   GitHub Releases page.
3. Verify the downloaded package in the directory containing `SHA256SUMS`:

   ```bash
   sha256sum --check SHA256SUMS --ignore-missing
   ```

4. Install the Debian package:

   ```bash
   sudo apt install ./path/to/webtop-manager.deb
   ```

   Or run the AppImage without installing it:

   ```bash
   chmod +x ./path/to/webtop-manager.AppImage
   ./path/to/webtop-manager.AppImage
   ```

The desktop remains launchable when Docker is missing or inaccessible and will
show a diagnostic state. Docker access must be fixed by the host administrator;
the application deliberately does not weaken host permissions.

## Architecture

```text
React WebView
  | fixed Tauri commands and redacted events
Tauri desktop process
  | mode-0600 Unix Socket, versioned /v1 API
Persistent controller container
  | local Docker Socket
Owned Webtops, frpc, isolated workers, SQLite state, and /config data
```

Only resources carrying the complete `com.cue.webtop-manager.*` ownership label
set are managed. The WebView cannot send shell fragments, arbitrary Docker JSON,
host paths for deletion, or generic URL-open requests. See
[docs/architecture.md](docs/architecture.md) for the detailed process and state
model.

## Repository layout

- `src`: React UI, localization, typed API client, and frontend tests.
- `src-tauri`: Docker diagnostics, controller bootstrap, native dialogs, and
  strict Tauri commands.
- `crates/contracts`: versioned, secret-free contracts and safety validation.
- `crates/controller`: persistent `/v1` API, SQLite state, Docker operations,
  templates, images, and FRP lifecycle.
- `crates/worker`: network-isolated snapshot, preflight, and restore worker.
- `scripts`: repeatable setup, diagnostics, checks, development, and packaging.
- `docs`: architecture, development, security, FRP, and implementation status.

## Development

Prerequisites are Linux x86_64, Docker Engine, Tauri 2 Linux system libraries,
Rust 1.88.0, Node.js 22.23.2, pnpm 10.4.1, and `zstd`.

On Ubuntu 24.04:

```bash
./scripts/setup-dev.sh --install-system-deps
./scripts/doctor.sh
./scripts/dev.sh
```

The setup script prompts for sudo only when system packages are missing. It
does not install Docker or change Docker access. See the
[development guide](docs/development.md) for toolchain details and troubleshooting.

Run all repository checks with:

```bash
./scripts/check.sh
cargo check --package webtop-manager --locked
./scripts/check-release-version.sh
```

To rebuild the embedded controller after controller, worker, or API changes:

```bash
./scripts/package-controller.sh
zstd --test --quiet src-tauri/assets/controller-image.tar.zst
```

The generated OCI archive is intentionally ignored by Git. GitHub Actions
rebuilds and verifies it before creating release packages.

## Release process

The release workflow is also manually dispatchable. A manual run builds and
uploads installation artifacts without creating a GitHub Release, making it the
recommended dress rehearsal before tagging.

For a tagged alpha release:

1. Move completed entries in `CHANGELOG.md` under the release version and date.
2. Update the version in `Cargo.toml`, `package.json`, and
   `src-tauri/tauri.conf.json`.
3. Run the local checks and verify the exact tag:

   ```bash
   ./scripts/check.sh
   cargo check --package webtop-manager --locked
   ./scripts/check-release-version.sh v0.1.0
   ```

4. Push `main`, run the `release` workflow manually, and install-test its
   artifact on a clean Ubuntu 24.04 x86_64 host with Docker.
5. Create and push the matching `v<version>` tag. The tag run publishes the
   `.deb`, AppImage, and `SHA256SUMS` as a GitHub pre-release.

The workflow rejects a tag that differs from the Cargo, frontend, or Tauri
version and grants write access only to the final publishing job.

## Contributing and security reports

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. Report
security vulnerabilities privately according to [SECURITY.md](SECURITY.md),
never through a public issue.

## License

Webtop Manager is licensed under the [MIT License](LICENSE). Third-party
components remain subject to their respective licenses.
