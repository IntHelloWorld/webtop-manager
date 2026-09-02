#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
project_version=$(awk '
  $0 == "[workspace.package]" { in_workspace_package = 1; next }
  /^\[/ { in_workspace_package = 0 }
  in_workspace_package && $1 == "version" {
    gsub(/["[:space:]]/, "", $3)
    print $3
    exit
  }
' "$project_root/Cargo.toml")
if [[ -z "$project_version" ]]; then
  echo "unable to read workspace package version" >&2
  exit 1
fi
image_ref="com.cue.webtop-manager/controller:$project_version"
asset_path="$project_root/src-tauri/assets/controller-image.tar.zst"
temporary_dir=$(mktemp -d)
trap 'rm -rf -- "$temporary_dir"' EXIT

docker build \
  --platform linux/amd64 \
  --file "$project_root/crates/controller/Dockerfile" \
  --tag "$image_ref" \
  "$project_root"

docker image save "$image_ref" --output "$temporary_dir/controller-image.tar"
zstd --quiet --threads=0 --force --ultra -19 \
  "$temporary_dir/controller-image.tar" \
  -o "$asset_path"
chmod 0644 "$asset_path"
sha256sum "$asset_path"
