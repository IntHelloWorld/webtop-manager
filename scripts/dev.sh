#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$project_root/scripts/dev-env.sh"
cd "$project_root"

if [[ "${1:-}" == "--with-controller" ]]; then
  "$project_root/scripts/package-controller.sh"
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--with-controller]" >&2
  exit 2
fi

exec pnpm tauri dev
