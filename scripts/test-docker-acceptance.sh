#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

webtop_image=${WEBTOP_ACCEPTANCE_IMAGE:-lscr.io/linuxserver/webtop:latest}
frpc_image='ghcr.io/fatedier/frpc:v0.70.1@sha256:e6483f2a916de67281597ba8fd03dc25d4f6fbd7ed0eafa042b2a5e4dcb5ee22'
frps_image='ghcr.io/fatedier/frps:v0.70.1@sha256:dab4febe235a24ddda5c20b1971ce34a31dc9f33983db3b126d278b932650408'

docker pull "$webtop_image"
docker pull "$frpc_image"
docker pull "$frps_image"

WEBTOP_OPERATION_TEST_IMAGE="$webtop_image" ./scripts/test-operation-recovery.sh

WEBTOP_TEMPLATE_TEST_IMAGE="$webtop_image" \
WEBTOP_TEMPLATE_REMOVE_SOURCE_IMAGE=1 \
  ./scripts/test-template-integration.sh

WEBTOP_FRP_TEST_IMAGE="$webtop_image" ./scripts/test-frp-integration.sh

echo "Docker-backed release acceptance passed"
