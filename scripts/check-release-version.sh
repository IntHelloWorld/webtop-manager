#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

cargo_version=$(awk '
  $0 == "[workspace.package]" { in_workspace_package = 1; next }
  /^\[/ { in_workspace_package = 0 }
  in_workspace_package && $1 == "version" {
    gsub(/["[:space:]]/, "", $3)
    print $3
    exit
  }
' Cargo.toml)

package_version=$(node -p "require('./package.json').version")
tauri_version=$(node -p "require('./src-tauri/tauri.conf.json').version")

if [[ -z "$cargo_version" ]]; then
  echo "unable to read Cargo workspace version" >&2
  exit 1
fi

if [[ "$package_version" != "$cargo_version" ]]; then
  echo "package.json version $package_version does not match Cargo version $cargo_version" >&2
  exit 1
fi

if [[ "$tauri_version" != "$cargo_version" ]]; then
  echo "tauri.conf.json version $tauri_version does not match Cargo version $cargo_version" >&2
  exit 1
fi

cargo metadata --format-version 1 --no-deps --locked >/dev/null

if [[ $# -gt 0 && -n "$1" ]]; then
  case "$1" in
    refs/heads/* | refs/pull/*) ;;
    refs/tags/* | v*)
      release_tag=${1#refs/tags/}
      expected_tag="v$cargo_version"
      if [[ "$release_tag" != "$expected_tag" ]]; then
        echo "release tag $release_tag does not match expected tag $expected_tag" >&2
        exit 1
      fi
      ;;
    *)
      echo "unsupported release ref $1" >&2
      exit 1
      ;;
  esac
fi

echo "release metadata is consistent for version $cargo_version"
