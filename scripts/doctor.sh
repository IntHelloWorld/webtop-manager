#!/usr/bin/env bash
set -uo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$project_root/scripts/dev-env.sh"

failures=0
warnings=0

pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; failures=$((failures + 1)); }
warn() { printf 'WARN  %s\n' "$1"; warnings=$((warnings + 1)); }

check_command() {
  local command_name=$1
  local description=$2
  if command -v "$command_name" >/dev/null 2>&1; then
    pass "$description: $($command_name --version 2>&1 | head -n 1)"
  else
    fail "$description is not installed"
  fi
}

check_command node "Node.js"
check_command pnpm "pnpm"
check_command rustc "Rust"
check_command cargo "Cargo"
check_command docker "Docker CLI"
check_command zstd "zstd"

if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists webkit2gtk-4.1; then
  pass "WebKitGTK development files: $(pkg-config --modversion webkit2gtk-4.1)"
else
  fail "WebKitGTK 4.1 development files are missing"
fi

if command -v cc >/dev/null 2>&1; then
  pass "C compiler: $(cc --version | head -n 1)"
else
  fail "C compiler is missing (install build-essential)"
fi

if [[ -S /var/run/docker.sock ]]; then
  if docker version --format '{{.Server.Version}}' >/dev/null 2>&1; then
    pass "Docker daemon is reachable"
  else
    fail "Docker socket exists but the daemon is not reachable"
  fi
else
  fail "Docker socket /var/run/docker.sock is missing"
fi

if [[ -f "$project_root/src-tauri/assets/controller-image.tar.zst" ]]; then
  pass "Bundled controller OCI asset exists"
else
  warn "Controller OCI asset is absent; development starts in diagnostic mode"
fi

printf '\nSummary: %d failure(s), %d warning(s)\n' "$failures" "$warnings"
if (( failures > 0 )); then
  exit 1
fi
