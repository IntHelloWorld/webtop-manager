#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$project_root/scripts/dev-env.sh"
cd "$project_root"

cargo fmt --all --check
cargo test --workspace --exclude webtop-manager
pnpm install --frozen-lockfile
pnpm test
pnpm build
