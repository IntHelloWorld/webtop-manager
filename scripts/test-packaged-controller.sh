#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

command -v curl >/dev/null
command -v docker >/dev/null
command -v jq >/dev/null

expected_version=${WEBTOP_EXPECTED_VERSION:-$(node -p "require('./package.json').version")}
controller_image=${WEBTOP_CONTROLLER_TEST_IMAGE:-com.cue.webtop-manager/controller:$expected_version}
test_root=$(mktemp -d /tmp/webtop-manager-package-probe.XXXXXX)
container_name="webtop-manager-package-probe-$$"

cleanup() {
  exit_status=$?
  if [[ $exit_status -ne 0 ]]; then
    docker container logs "$container_name" >&2 2>/dev/null || true
  fi
  docker container rm --force "$container_name" >/dev/null 2>&1 || true
  find "$test_root" -depth -delete >/dev/null 2>&1 || true
}
trap cleanup EXIT

mkdir -p \
  "$test_root/state" \
  "$test_root/environments" \
  "$test_root/snapshots" \
  "$test_root/staging" \
  "$test_root/runtime"

socket_gid=$(stat -c '%g' /var/run/docker.sock)
docker run --detach \
  --name "$container_name" \
  --label com.cue.webtop-manager.owner=managed \
  --label com.cue.webtop-manager.resource-kind=controller-probe \
  --network none \
  --read-only \
  --security-opt no-new-privileges=true \
  --cap-drop ALL \
  --group-add "$socket_gid" \
  --user "$(id -u):$(id -g)" \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
  --mount type=bind,src="$test_root/state",dst=/state \
  --mount type=bind,src="$test_root/environments",dst=/data/environments \
  --mount type=bind,src="$test_root/snapshots",dst=/data/snapshots \
  --mount type=bind,src="$test_root/staging",dst=/data/staging \
  --mount type=bind,src="$test_root/runtime",dst=/run/webtop-manager \
  --env "WEBTOP_MANAGER_HOST_ENVIRONMENT_ROOT=$test_root/environments" \
  "$controller_image" >/dev/null

health=""
for _ in $(seq 1 40); do
  if [[ -S "$test_root/runtime/controller.sock" ]]; then
    health=$(curl --silent --show-error \
      --unix-socket "$test_root/runtime/controller.sock" \
      http://localhost/v1/health) && break
  fi
  sleep 0.25
done

jq -e --arg version "$expected_version" '
  .controllerVersion == $version
  and (.capabilities | index("durable_image_pull_v1"))
  and (.capabilities | index("controller_schema_v1"))
' <<<"$health" >/dev/null
[[ $(stat -c '%a' "$test_root/runtime/controller.sock") == 600 ]]

echo "packaged controller Unix-socket health probe passed"
