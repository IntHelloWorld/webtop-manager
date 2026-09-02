#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
node_version="22.23.2"
node_archive="node-v${node_version}-linux-x64.tar.xz"
node_checksum="d60acfe00a2932254bb0ad20e01b0d74397a0875595de719654b214f4b03f307"
node_url="https://nodejs.org/dist/v${node_version}/${node_archive}"
rust_version="1.88.0"
pnpm_version="10.4.1"
user_bin="${HOME}/.local/bin"
data_root="${HOME}/.local/share/webtop-manager-dev"
cache_root="${HOME}/.cache/webtop-manager-dev"
node_root="${data_root}/node-v${node_version}-linux-x64"

system_packages=(
  build-essential
  file
  libayatana-appindicator3-dev
  librsvg2-dev
  libssl-dev
  libwebkit2gtk-4.1-dev
  libxdo-dev
  patchelf
  pkg-config
  zstd
)

install_system=false
if [[ "${1:-}" == "--install-system-deps" ]]; then
  install_system=true
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--install-system-deps]" >&2
  exit 2
fi

ubuntu_updates_enabled() {
  local codename=$1
  local suite="${codename}-updates"
  local source_files=()

  shopt -s nullglob
  source_files=(
    /etc/apt/sources.list
    /etc/apt/sources.list.d/*.list
    /etc/apt/sources.list.d/*.sources
  )
  shopt -u nullglob

  (( ${#source_files[@]} > 0 )) || return 1

  awk -v suite="$suite" '
    /^[[:space:]]*#/ { next }
    $1 == "Suites:" || $1 == "deb" {
      for (field = 2; field <= NF; field++) {
        if ($field == suite) {
          found = 1
        }
      }
    }
    END { exit(found ? 0 : 1) }
  ' "${source_files[@]}"
}

missing_packages=()
for package in "${system_packages[@]}"; do
  if ! dpkg-query -W -f='${db:Status-Abbrev}' "$package" 2>/dev/null | grep -q '^ii'; then
    missing_packages+=("$package")
  fi
done

if (( ${#missing_packages[@]} > 0 )); then
  if [[ "$install_system" == true ]]; then
    ubuntu_codename=""
    if [[ -r /etc/os-release ]]; then
      ubuntu_codename=$(sed -n 's/^VERSION_CODENAME=//p' /etc/os-release)
    fi
    if [[ -n "$ubuntu_codename" ]] && ! ubuntu_updates_enabled "$ubuntu_codename"; then
      cat >&2 <<EOF
Ubuntu suite '${ubuntu_codename}-updates' is not enabled.
This can make runtime and -dev package versions disagree.
Enable '${ubuntu_codename} ${ubuntu_codename}-updates ${ubuntu_codename}-backports'
in /etc/apt/sources.list.d/ubuntu.sources, then run this command again.
See docs/development.md#apt-依赖版本不匹配.
EOF
      exit 1
    fi
    sudo apt-get update
    sudo apt-get install -y "${missing_packages[@]}"
  else
    echo "Missing Ubuntu system packages: ${missing_packages[*]}" >&2
    echo "Run: ./scripts/setup-dev.sh --install-system-deps" >&2
  fi
fi

mkdir -p "$user_bin" "$data_root" "$cache_root"

if [[ ! -x "$node_root/bin/node" ]]; then
  archive_path="${cache_root}/${node_archive}"
  curl -fL "$node_url" -o "$archive_path"
  printf '%s  %s\n' "$node_checksum" "$archive_path" | sha256sum --check
  tar -xJf "$archive_path" -C "$data_root"
fi

ensure_link() {
  local source_path=$1
  local target_path=$2
  if [[ -L "$target_path" && "$(readlink -f "$target_path")" == "$(readlink -f "$source_path")" ]]; then
    return
  fi
  if [[ -e "$target_path" || -L "$target_path" ]]; then
    echo "Refusing to replace existing tool: $target_path" >&2
    exit 1
  fi
  ln -s "$source_path" "$target_path"
}

ensure_link "$node_root/bin/node" "$user_bin/node"
ensure_link "$node_root/bin/npm" "$user_bin/npm"
ensure_link "$node_root/bin/npx" "$user_bin/npx"
ensure_link "$node_root/bin/corepack" "$user_bin/corepack"

export PATH="$user_bin:${HOME}/.cargo/bin:$PATH"
corepack install --global "pnpm@${pnpm_version}"
corepack enable --install-directory "$user_bin" pnpm

if [[ ! -x "${HOME}/.cargo/bin/rustup" ]]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain "$rust_version" --profile default --no-modify-path
fi
"${HOME}/.cargo/bin/rustup" toolchain install "$rust_version" --profile default
"${HOME}/.cargo/bin/rustup" default "$rust_version"

cd "$project_root"
CI=1 pnpm install --frozen-lockfile

echo
echo "User toolchains are ready."
echo "Open a new terminal or run: source ./scripts/dev-env.sh"
if (( ${#missing_packages[@]} > 0 )) && [[ "$install_system" == false ]]; then
  echo "System dependencies are still missing; run the sudo command shown above."
fi
