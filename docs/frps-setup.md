# Remote frps setup

[简体中文](frps-setup.zh-CN.md)

Webtop Manager generates an isolated frps deployment for two remote Linux
server scenarios. Save the server address, public address, and frpc image in
**Server settings**, then select **Open frps setup guide**. In the guide, choose
the frps `bindPort` and remote port range, then select **Generate commands**.
The selected ports are saved back to the client settings before the commands
are requested, so frpc and frps use the same values. Editing a port afterward
hides the old commands until they are generated again. Neither deployment path
reads, edits, stops, or restarts an unrelated existing frps service. Re-running
a command updates and restarts only the dedicated instance previously created
and marked as managed by Webtop Manager.

## New server with Docker

The Docker script:

- creates `/opt/webtop-manager-frps/frps.toml` and a mode-`0600` token file;
- runs the official `ghcr.io/fatedier/frps:v0.70.1` image in a dedicated
  `webtop-manager-frps` container with host networking;
- labels the container as Webtop Manager-owned and refuses to replace a
  same-named container without those management labels;
- configures the selected `bindPort` and `allowPorts` range;
- installs the same automatically generated authentication token used by the
  local frpc service; and
- prints the initial frps logs after startup.

## New server without Docker

The native installation script supports Linux x86_64 and arm64. It downloads a
pinned frp release archive, verifies its SHA-256 checksum, installs `frps` under
`/opt/webtop-manager-frps-native`, and creates the restricted `webtop-frps`
system user. Configuration and the mode-`0600` token file live under
`/etc/webtop-manager-frps`; the dedicated `webtop-manager-frps.service` systemd
unit starts the service and enables it at boot. The script validates the
configuration with `frps verify`, enables the unit, and explicitly restarts it
so a repaired token is active immediately. It refuses to
overwrite a same-named unit or configuration directory that lacks the Webtop
Manager marker.

The remote server must use systemd and provide `curl`, `tar`, and `sha256sum`.
Other architectures require manually downloading the matching archive from the
official frp release page and adapting the same service configuration.

## Shared port safety

Process and filesystem isolation cannot make two services bind the same network
ports. Before running either script, choose an unused frps bind port and a
remote port range that does not overlap any existing frps proxy. The generated
scripts never modify an existing service to resolve a conflict; the dedicated
Webtop Manager service will fail to start or publish until its ports are changed.

Generated installation and token commands contain the authentication token.
Run them only in a trusted server terminal and do not save them in tickets,
chat messages, shell scripts committed to source control, or public logs.

## Token persistence and recovery

The app generates the token once and keeps it under its per-user application
data directory. Normal upgrades, package removal and reinstallation do not
delete that data, so they reuse the existing token. There is no routine token
rotation control.

The controller stores only a SHA-256 fingerprint in SQLite. If the protected
token file is deleted, replaced, or emptied, it does not silently generate a
new token or start frpc. Server settings instead expose a recovery flow. That
flow creates one stable replacement credential, keeps reusing it across
retries, and asks the user to rerun the managed frps command. A successful
authenticated connectivity test completes recovery. If the original token was
backed up, restoring that exact mode-`0600` file avoids remote reconfiguration.

## Firewall rules

The generated command deliberately does not modify firewall rules. Allow the
configured frps bind port from the client machine and allow the configured
remote TCP port range from intended Webtop users. Apply both cloud-provider
security-group rules and host firewall rules where applicable.

## Verification

Return to **Server settings** and select **Test connectivity**. The test starts a
temporary, restricted frpc container and performs a real authenticated login to
frps. A successful result verifies DNS/address resolution, TCP reachability, and
the shared authentication token. It does not verify every remote publication
port; those still depend on `allowPorts` and firewall rules.

After the test succeeds, start the persistent frpc service from the same page.
The status panel supports start, restart, stop, and periodic Docker status
refreshes.
