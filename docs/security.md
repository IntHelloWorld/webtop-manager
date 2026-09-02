# Security model

[简体中文](security.zh-CN.md)

Docker Socket access is root-equivalent. The controller is therefore treated
as a privileged local service even though its container uses a read-only root
filesystem, no published ports, `no-new-privileges`, no Linux capabilities and
a minimal set of bind mounts.

## Enforced invariants

- No generic shell, exec, Docker, path-delete or URL-open command is available to the WebView.
- `privileged=true`, reserved `/config` mounts and secret environment keys are rejected.
- The Docker Socket can only be mounted through the explicit dangerous toggle.
- Webtop passwords are random and mounted from a `0600` file through `FILE__PASSWORD`.
- Official-image deletion accepts only the documented allowlist, never forces
  removal, and refuses while an image pull runs or any running/stopped container
  still uses the image. Cache pruning and tagged-image deletion serialize with
  pull registration to avoid deletion/download races.
- Secrets are excluded from SQLite records, events and normal API values. Two
  dedicated read paths are explicit exceptions: `/v1/frps/setup` returns a
  copyable server command containing the FRP token after the user opens the
  setup guide, while `/v1/environments/{id}/credentials` reads the protected
  Webtop password only for an environment currently published to the Internet.
  The environment card masks that password by default.
- The FRP token is generated once and tracked in SQLite only by SHA-256
  fingerprint. A missing or replaced secret suspends automatic frpc startup;
  the recovery endpoint is rejected while the original token is healthy and a
  recovery credential becomes active only after an authenticated test succeeds.
- Application data deletion requires exact-name confirmation and a canonical path strictly below the environment root.
- User-supplied external mounts are never deleted automatically.
- Environment data-directory opening accepts only the managed `/config` mount
  for the requested UUID, canonicalizes it, and verifies that it remains below
  the application environment root. The exact legacy `/data/environments`
  layout is accepted only for environments created before the host-path fix.
- Snapshots are written as `0600` temporary files, SHA-256 checked, then atomically renamed.
- Snapshot workers accept absolute paths only, skip sockets/FIFOs/devices and reject archive traversal.
- A template is a Docker commit plus a separate complete `/config` `tar.zst`.
  The commit preserves the immutable source image's boot metadata while removing
  instance identity and secret-like environment defaults. Mounted `/config`
  data is never assumed to be part of the image.
- Template operation output is a bounded set of controller-generated, path-free
  status lines. Passwords, tokens, arbitrary host paths, and container command
  output are never copied into the UI log.
- Template import/export paths never cross the WebView boundary. Native dialogs
  run asynchronously in the Tauri backend, copy on background threads with
  byte progress and mode `0600`, and pass only UUID staging identifiers to
  `/v1`. The controller resolves those UUIDs below its dedicated staging root.
- Import snapshot publication copies into a `0600` temporary file inside the
  snapshot filesystem before atomic rename, so separate staging and snapshot
  mounts do not rely on a cross-filesystem rename.
- Cancelling template import/export is cooperative. The controller reports
  `cancelled` only after removing partial packages, extraction directories,
  unpublished snapshots, and temporary imported image tags. Native desktop
  copies likewise remove their partial destination before returning.
- `.wtmpl` imports allow exactly `manifest.json`, `payload/image.tar.zst`, and
  `payload/config.tar.zst`. Validation checks schema, Linux/amd64 platform,
  path safety, sizes, SHA-256 values, a single Docker image and the expected
  internal tag before Docker load. The saved-image config is rewritten to a
  unique staging tag and local ownership labels so a package cannot overwrite
  or impersonate an existing app image.
- Template packages and snapshots are not encrypted. They can contain SSH keys,
  browser profiles, cloud credentials, and other sensitive data from `/config`;
  the save, import, and export flows require explicit warnings.
- Image pulls, cache pruning, commits, imports, exports and owned-image deletion
  share controller-side resource locks. Environment lifecycle and deletion are
  rejected while that environment is being snapshotted or restored.

## Internet exposure warning

Webtop uses HTTPS on port 3001 with a self-signed certificate. Its current
official documentation states that the optional built-in basic authentication
is suitable only on a trusted local network and recommends a robust reverse
proxy for Internet exposure. The product specification intentionally limits v1
to personal-use TCP forwarding and does not provide that stronger authentication
layer. The UI keeps this risk visible; publication is disabled by default and
must be enabled explicitly while creating an environment. Published cards show
the assigned URL and whether the shared frpc client is connected.

References: [LinuxServer Webtop](https://docs.linuxserver.io/images/docker-webtop/),
[Tauri capabilities](https://v2.tauri.app/security/capabilities/), and
[frp tokenSource](https://gofrp.org/en/docs/features/common/authentication/).
