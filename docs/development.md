# Development environment and startup guide

[简体中文](development.zh-CN.md)

This guide applies to the project's currently supported platform: Ubuntu 24.04,
Linux x86_64, and a local Docker Engine. The project does not install Docker,
change Docker group membership, or modify permissions on
`/var/run/docker.sock` automatically.

## Pinned versions

| Tool | Version |
| --- | --- |
| Rust | 1.88.0 |
| Node.js | 22.23.2 |
| pnpm | 10.4.1 |
| Tauri | Pinned by `Cargo.lock` |
| Frontend dependencies | Pinned by `pnpm-lock.yaml` |

The Node.js binary is installed under
`~/.local/share/webtop-manager-dev/node-v22.23.2-linux-x64` and exposed through
`~/.local/bin`. Downloads are checked against the official Node.js SHA-256
digest. Rust is installed with the official rustup installer under the default
`~/.rustup` and `~/.cargo` paths.

## One-time setup

Run this command from the project root:

```bash
./scripts/setup-dev.sh --install-system-deps
```

The system-dependency step calls `sudo apt-get` and prompts for the local
administrator password. The script then configures Rust, Node.js, and pnpm
idempotently and runs `pnpm install --frozen-lockfile`. Omit the option if you
do not want the script to invoke sudo; it will list missing packages without
changing the system.

New terminals automatically receive `~/.local/bin` and `~/.cargo/bin` through
`~/.profile`. To load them in the current terminal immediately, run:

```bash
source ./scripts/dev-env.sh
```

## Environment diagnostics

```bash
./scripts/doctor.sh
```

The diagnostic covers Node.js, pnpm, Rust, Cargo, the Docker CLI and daemon, a C
compiler, WebKitGTK 4.1, zstd, and the controller OCI. A missing controller OCI
is only a warning because diagnostic mode can run independently.

## Start development mode

To start only the desktop UI and Docker diagnostics:

```bash
./scripts/dev.sh
```

`pnpm tauri dev` starts the Vite development server according to
`src-tauri/tauri.conf.json`; there is no need to run `pnpm dev` in another
terminal. React and CSS changes support hot reload, while Rust and Tauri changes
trigger a desktop-process rebuild.

To build and embed the controller OCI as well:

```bash
./scripts/dev.sh --with-controller
```

The first run builds the static Rust controller and worker, saves the Docker
image, and compresses it with zstd, so it takes considerably longer than a
normal startup. The script creates only
`com.cue.webtop-manager/controller:1.0.0` and project asset files; it does not
import or modify existing Compose environments.

## Common verification commands

```bash
./scripts/check.sh
```

This script runs Rust formatting checks, core Rust tests, a lockfile-based
dependency installation, frontend tests, and a production build. Run the full
Tauri check separately:

```bash
cargo check --package webtop-manager
```

## Troubleshooting

### `cargo`, `node`, or `pnpm` is not found

Open a new terminal or run:

```bash
source ./scripts/dev-env.sh
```

### `webkit2gtk-4.1` is missing

The system dependencies have not been installed. Run:

```bash
./scripts/setup-dev.sh --install-system-deps
```

### APT dependency versions do not match

If installation reports `Depends: ... (= older version) but newer version is
to be installed`, first check whether the Ubuntu updates repository is disabled:

```bash
apt-mark showhold
apt-cache policy libgtk-3-0t64 libgtk-3-dev zlib1g zlib1g-dev
```

On Ubuntu 24.04, `/etc/apt/sources.list.d/ubuntu.sources` should enable these
suites for the main Ubuntu mirror:

```text
Suites: noble noble-updates noble-backports
```

Edit the line with `sudoedit /etc/apt/sources.list.d/ubuntu.sources`; do not
downgrade already installed runtime libraries. Refresh the package index and
try again:

```bash
sudo apt-get update
./scripts/setup-dev.sh --install-system-deps
```

The setup script checks for `noble-updates` before invoking APT. If it is
missing, the script explains the problem but does not edit system repositories.

### The UI reports that the controller image is missing

This is the expected diagnostic state when the OCI has not been generated. To
enable environment management, run:

```bash
./scripts/package-controller.sh
./scripts/dev.sh
```

### Docker permission denied

The application diagnoses access but never runs `usermod` or changes Socket
permissions. Ask a system administrator to configure Docker access according to
the host security policy, then sign in again.

### Startup reports `ENOSPC: System limit for number of file watchers reached`

The Vite configuration excludes the Rust `target/` build directories to avoid
watching a large number of build artifacts. If other development tools still
exhaust the system watcher limit, close unused development servers or IDE
workspaces before running `./scripts/dev.sh` again. The project does not modify
the system-wide inotify limit automatically.
