use std::fmt::Write;

use webtop_contracts::{ServerSettings, DEFAULT_FRPS_IMAGE};

const FRP_VERSION: &str = "0.70.1";
const FRP_LINUX_AMD64_SHA256: &str =
    "333da23d1b9009d7c01638e9ba38cf4600f7d37d393f854e96ee1396adefa9a6";
const FRP_LINUX_ARM64_SHA256: &str =
    "3990f396a9a490ee7f0e5f355287750ed41520064ed999eab443b5e9a78d773d";

#[derive(Debug, Clone)]
pub struct Proxy {
    pub resource_id: String,
    pub local_port: u16,
    pub remote_port: u16,
}

/// Renders a secret-free frpc configuration. The token is loaded by frpc from a
/// separately mounted 0600 file.
pub fn render(settings: &ServerSettings, proxies: &[Proxy]) -> String {
    render_with_login_behavior(settings, proxies, false)
}

pub fn render_connectivity_test(settings: &ServerSettings) -> String {
    render_with_login_behavior(settings, &[], true)
}

fn render_with_login_behavior(
    settings: &ServerSettings,
    proxies: &[Proxy],
    login_fail_exit: bool,
) -> String {
    let mut output = format!(
        "serverAddr = {:?}\nserverPort = {}\nloginFailExit = {}\nlog.to = \"console\"\nlog.level = \"info\"\nauth.method = \"token\"\nauth.tokenSource.type = \"file\"\nauth.tokenSource.file.path = \"/run/webtop-manager/frp-token\"\n\n",
        settings.frps_host, settings.frps_port, login_fail_exit
    );
    for proxy in proxies {
        let _ = writeln!(output, "[[proxies]]");
        let _ = writeln!(
            output,
            "name = {:?}",
            format!("webtop-{}", proxy.resource_id)
        );
        let _ = writeln!(output, "type = \"tcp\"");
        let _ = writeln!(output, "localIP = \"127.0.0.1\"");
        let _ = writeln!(output, "localPort = {}", proxy.local_port);
        let _ = writeln!(output, "remotePort = {}\n", proxy.remote_port);
    }
    output
}

/// Produces explicit, copyable setup material containing the token. These
/// values are exposed only through the dedicated tutorial API.
pub fn render_frps_docker_setup_script(settings: &ServerSettings, token: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

FRPS_DIR=/opt/webtop-manager-frps
CONTAINER_NAME=webtop-manager-frps
OWNER_LABEL=com.cue.webtop-manager.owner
KIND_LABEL=com.cue.webtop-manager.resource-kind

EXISTING_ID="$(sudo docker ps -aq --filter "name=^/${{CONTAINER_NAME}}$")"
MANAGED_ID="$(sudo docker ps -aq \
  --filter "name=^/${{CONTAINER_NAME}}$" \
  --filter "label=${{OWNER_LABEL}}=managed" \
  --filter "label=${{KIND_LABEL}}=frps")"
if [ -n "$EXISTING_ID" ] && [ "$EXISTING_ID" != "$MANAGED_ID" ]; then
  echo "Refusing to replace an existing container not managed by Webtop Manager: $CONTAINER_NAME" >&2
  exit 1
fi

sudo install -d -m 700 "$FRPS_DIR"

sudo tee "$FRPS_DIR/frps.toml" >/dev/null <<'FRPS_CONFIG'
bindPort = {bind_port}
auth.method = "token"
auth.tokenSource.type = "file"
auth.tokenSource.file.path = "/etc/frp/frp-token"
allowPorts = [
  {{ start = {port_start}, end = {port_end} }}
]
FRPS_CONFIG

sudo tee "$FRPS_DIR/frp-token" >/dev/null <<'FRPS_TOKEN'
{token}
FRPS_TOKEN
sudo chmod 600 "$FRPS_DIR/frps.toml" "$FRPS_DIR/frp-token"

sudo docker pull {image}
if [ -n "$MANAGED_ID" ]; then
  sudo docker rm -f "$MANAGED_ID"
fi
sudo docker run -d \
  --name "$CONTAINER_NAME" \
  --label "${{OWNER_LABEL}}=managed" \
  --label "${{KIND_LABEL}}=frps" \
  --restart unless-stopped \
  --network host \
  --security-opt no-new-privileges:true \
  --cap-drop ALL \
  -v "$FRPS_DIR/frps.toml:/etc/frp/frps.toml:ro" \
  -v "$FRPS_DIR/frp-token:/etc/frp/frp-token:ro" \
  {image} -c /etc/frp/frps.toml

sudo docker logs --tail 50 "$CONTAINER_NAME"
"#,
        bind_port = settings.frps_port,
        port_start = settings.remote_port_start,
        port_end = settings.remote_port_end,
        image = DEFAULT_FRPS_IMAGE,
    )
}

pub fn render_frps_native_setup_script(settings: &ServerSettings, token: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

FRP_VERSION={version}
SERVICE_NAME=webtop-manager-frps.service
SERVICE_USER=webtop-frps
INSTALL_DIR=/opt/webtop-manager-frps-native
CONFIG_DIR=/etc/webtop-manager-frps
UNIT_FILE="/etc/systemd/system/$SERVICE_NAME"

if sudo test -e "$UNIT_FILE" && \
  ! sudo grep -Fqx '# Managed by Webtop Manager' "$UNIT_FILE"; then
  echo "Refusing to replace an existing systemd unit not managed by Webtop Manager: $UNIT_FILE" >&2
  exit 1
fi
if sudo test -d "$CONFIG_DIR" && \
  ! sudo test -f "$CONFIG_DIR/.managed-by-webtop-manager"; then
  echo "Refusing to replace an existing configuration directory: $CONFIG_DIR" >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64|amd64)
    FRP_ARCH=amd64
    FRP_SHA256={amd64_sha256}
    ;;
  aarch64|arm64)
    FRP_ARCH=arm64
    FRP_SHA256={arm64_sha256}
    ;;
  *)
    echo "Unsupported architecture: $(uname -m). Download the matching frp release manually." >&2
    exit 1
    ;;
esac

command -v curl >/dev/null || {{ echo "curl is required" >&2; exit 1; }}
command -v tar >/dev/null || {{ echo "tar is required" >&2; exit 1; }}
command -v systemctl >/dev/null || {{ echo "systemd is required" >&2; exit 1; }}

FRP_ARCHIVE="frp_${{FRP_VERSION}}_linux_${{FRP_ARCH}}.tar.gz"
FRP_URL="https://github.com/fatedier/frp/releases/download/v${{FRP_VERSION}}/${{FRP_ARCHIVE}}"
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf -- "$TEMP_DIR"' EXIT

curl --fail --location --proto '=https' --tlsv1.2 \
  "$FRP_URL" -o "$TEMP_DIR/$FRP_ARCHIVE"
printf '%s  %s\n' "$FRP_SHA256" "$TEMP_DIR/$FRP_ARCHIVE" | sha256sum --check --status
tar -xzf "$TEMP_DIR/$FRP_ARCHIVE" -C "$TEMP_DIR"

if ! getent group "$SERVICE_USER" >/dev/null; then
  sudo groupadd --system "$SERVICE_USER"
fi
if ! id -u "$SERVICE_USER" >/dev/null 2>&1; then
  sudo useradd --system --gid "$SERVICE_USER" \
    --home-dir /var/lib/webtop-manager-frps --create-home \
    --shell /usr/sbin/nologin "$SERVICE_USER"
fi

sudo install -d -o root -g root -m 0755 "$INSTALL_DIR/bin"
sudo install -o root -g root -m 0755 \
  "$TEMP_DIR/frp_${{FRP_VERSION}}_linux_${{FRP_ARCH}}/frps" "$INSTALL_DIR/bin/frps"
sudo install -d -o root -g "$SERVICE_USER" -m 0750 "$CONFIG_DIR"
sudo touch "$CONFIG_DIR/.managed-by-webtop-manager"
sudo chown root:root "$CONFIG_DIR/.managed-by-webtop-manager"
sudo chmod 0644 "$CONFIG_DIR/.managed-by-webtop-manager"

sudo tee "$CONFIG_DIR/frps.toml" >/dev/null <<'FRPS_CONFIG'
bindPort = {bind_port}
auth.method = "token"
auth.tokenSource.type = "file"
auth.tokenSource.file.path = "/etc/webtop-manager-frps/frp-token"
allowPorts = [
  {{ start = {port_start}, end = {port_end} }}
]
FRPS_CONFIG

sudo tee "$CONFIG_DIR/frp-token" >/dev/null <<'FRPS_TOKEN'
{token}
FRPS_TOKEN
sudo chown "root:$SERVICE_USER" "$CONFIG_DIR/frps.toml"
sudo chmod 0640 "$CONFIG_DIR/frps.toml"
sudo chown "$SERVICE_USER:$SERVICE_USER" "$CONFIG_DIR/frp-token"
sudo chmod 0600 "$CONFIG_DIR/frp-token"

sudo tee "$UNIT_FILE" >/dev/null <<'FRPS_SERVICE'
# Managed by Webtop Manager
[Unit]
Description=FRP server for Webtop Manager
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=webtop-frps
Group=webtop-frps
ExecStart=/opt/webtop-manager-frps-native/bin/frps -c /etc/webtop-manager-frps/frps.toml
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
ProtectHome=true
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
FRPS_SERVICE

sudo -u "$SERVICE_USER" "$INSTALL_DIR/bin/frps" verify -c "$CONFIG_DIR/frps.toml"
sudo systemctl daemon-reload
sudo systemctl enable "$SERVICE_NAME"
sudo systemctl restart "$SERVICE_NAME"
sudo systemctl --no-pager --full status "$SERVICE_NAME"
"#,
        version = FRP_VERSION,
        amd64_sha256 = FRP_LINUX_AMD64_SHA256,
        arm64_sha256 = FRP_LINUX_ARM64_SHA256,
        bind_port = settings.frps_port,
        port_start = settings.remote_port_start,
        port_end = settings.remote_port_end,
    )
}

#[cfg(test)]
mod tests {
    use webtop_contracts::ServerTokenState;

    use super::*;

    #[test]
    fn token_never_appears_in_generated_config() {
        let settings = ServerSettings {
            frps_host: "frps.example".into(),
            frps_port: 7000,
            public_ip: "203.0.113.1".into(),
            remote_port_start: 41000,
            remote_port_end: 42000,
            token_configured: true,
            token_state: ServerTokenState::Ready,
            frpc_image: "ghcr.io/fatedier/frpc:v0.70.1".into(),
        };
        let config = render(
            &settings,
            &[Proxy {
                resource_id: "abc".into(),
                local_port: 49152,
                remote_port: 41000,
            }],
        );
        assert!(config.contains("auth.tokenSource.type = \"file\""));
        assert!(config.contains("loginFailExit = false"));
        assert!(!config.contains("auth.token ="));
        assert!(config.contains("remotePort = 41000"));
    }

    #[test]
    fn connectivity_test_exits_after_the_first_login_failure() {
        let settings = ServerSettings {
            frps_host: "frps.example".into(),
            frps_port: 7000,
            public_ip: "203.0.113.1".into(),
            remote_port_start: 41000,
            remote_port_end: 42000,
            token_configured: true,
            token_state: ServerTokenState::Ready,
            frpc_image: "ghcr.io/fatedier/frpc:v0.70.1".into(),
        };
        assert!(render_connectivity_test(&settings).contains("loginFailExit = true"));
    }

    #[test]
    fn setup_script_matches_server_settings() {
        let settings = ServerSettings {
            frps_host: "frps.example".into(),
            frps_port: 7443,
            public_ip: "203.0.113.1".into(),
            remote_port_start: 41000,
            remote_port_end: 42000,
            token_configured: true,
            token_state: ServerTokenState::Ready,
            frpc_image: "ghcr.io/fatedier/frpc:v0.70.1".into(),
        };
        let script = render_frps_docker_setup_script(&settings, "generated-token");
        assert!(script.contains("bindPort = 7443"));
        assert!(script.contains("start = 41000, end = 42000"));
        assert!(script.contains("generated-token"));
        assert!(script.contains(DEFAULT_FRPS_IMAGE));
    }

    #[test]
    fn native_setup_is_pinned_and_managed_by_systemd() {
        let settings = ServerSettings {
            frps_host: "frps.example".into(),
            frps_port: 7443,
            public_ip: "203.0.113.1".into(),
            remote_port_start: 41000,
            remote_port_end: 42000,
            token_configured: true,
            token_state: ServerTokenState::Ready,
            frpc_image: "ghcr.io/fatedier/frpc:v0.70.1".into(),
        };
        let script = render_frps_native_setup_script(&settings, "generated-token");
        assert!(script.contains("FRP_VERSION=0.70.1"));
        assert!(script.contains(FRP_LINUX_AMD64_SHA256));
        assert!(script.contains(FRP_LINUX_ARM64_SHA256));
        assert!(script.contains("SERVICE_NAME=webtop-manager-frps.service"));
        assert!(script.contains("INSTALL_DIR=/opt/webtop-manager-frps-native"));
        assert!(script.contains("CONFIG_DIR=/etc/webtop-manager-frps"));
        assert!(!script.contains("/etc/systemd/system/frps.service"));
        assert!(!script.contains("/usr/local/bin/frps"));
        assert!(script.contains("generated-token"));
        assert!(script.contains("systemctl restart \"$SERVICE_NAME\""));
    }

    #[test]
    fn docker_setup_replaces_only_a_labeled_managed_container() {
        let settings = ServerSettings {
            frps_host: "frps.example".into(),
            frps_port: 7443,
            public_ip: "203.0.113.1".into(),
            remote_port_start: 41000,
            remote_port_end: 42000,
            token_configured: true,
            token_state: ServerTokenState::Ready,
            frpc_image: "ghcr.io/fatedier/frpc:v0.70.1".into(),
        };
        let script = render_frps_docker_setup_script(&settings, "generated-token");
        assert!(script.contains("com.cue.webtop-manager.owner"));
        assert!(script.contains("Refusing to replace an existing container"));
        assert!(script.contains("docker rm -f \"$MANAGED_ID\""));
        assert!(!script.contains("docker rm -f webtop-manager-frps"));
    }
}
