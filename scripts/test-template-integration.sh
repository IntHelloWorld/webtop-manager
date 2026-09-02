#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_root=$(mktemp -d)
socket_path="$test_root/controller.sock"
controller_pid=""
source_environment_id=""
derived_environment_id=""
template_id=""
imported_template_id=""
source_image_id=""
source_repo_digest=""
test_image=${WEBTOP_TEMPLATE_TEST_IMAGE:-lscr.io/linuxserver/webtop:latest}

api() {
  local method=$1
  local path=$2
  local body=${3-}
  if [[ -n "$body" ]]; then
    curl --connect-timeout 3 --max-time 120 --fail --silent --show-error --unix-socket "$socket_path" -X "$method" -H 'content-type: application/json' --data "$body" "http://localhost$path"
  else
    curl --connect-timeout 3 --max-time 120 --fail --silent --show-error --unix-socket "$socket_path" -X "$method" "http://localhost$path"
  fi
}

restore_source_image() {
  if docker image inspect "$test_image" >/dev/null 2>&1; then
    return
  fi
  if [[ -n "$source_repo_digest" ]]; then
    docker pull "$source_repo_digest" >/dev/null
    docker image tag "$source_repo_digest" "$test_image"
  elif [[ -f "$test_root/source-image.tar" ]]; then
    docker load --input "$test_root/source-image.tar" >/dev/null
  fi
}

wait_operation() {
  local operation_id=$1
  local response phase
  for _ in $(seq 1 300); do
    response=$(api GET "/v1/operations/$operation_id")
    phase=$(jq -r .phase <<<"$response")
    case "$phase" in
      succeeded) jq -c . <<<"$response"; return 0 ;;
      failed|cancelled|retryable) jq . <<<"$response" >&2; return 1 ;;
    esac
    sleep 0.2
  done
  echo "operation timed out: $operation_id" >&2
  return 1
}

cleanup() {
  local exit_status=$?
  set +e
  if [[ -n "$derived_environment_id" ]]; then
    api DELETE "/v1/environments/$derived_environment_id" "{\"confirmationName\":\"integration-restored\",\"deleteData\":true}" >/dev/null
  fi
  if [[ -n "$source_environment_id" ]]; then
    api DELETE "/v1/environments/$source_environment_id" "{\"confirmationName\":\"integration-source\",\"deleteData\":true}" >/dev/null
  fi
  if [[ -n "$controller_pid" ]]; then kill "$controller_pid" 2>/dev/null; wait "$controller_pid" 2>/dev/null; fi
  for id in "$source_environment_id" "$derived_environment_id"; do
    [[ -n "$id" ]] || continue
    container=$(docker container ls -aq --filter "label=com.cue.webtop-manager.resource-id=$id" | head -1)
    [[ -n "$container" ]] && docker container rm -f "$container" >/dev/null
  done
  for id in "$template_id" "$imported_template_id"; do
    [[ -n "$id" ]] || continue
    docker image rm "com.cue.webtop-manager/template:$id" >/dev/null 2>&1
    docker image rm "com.cue.webtop-manager/import-staging:$id" >/dev/null 2>&1
  done
  if [[ ${WEBTOP_TEMPLATE_REMOVE_SOURCE_IMAGE:-0} == 1 ]] \
    && ! docker image inspect "$test_image" >/dev/null 2>&1; then
    restore_source_image >/dev/null 2>&1 || true
  fi
  if [[ $exit_status -ne 0 && -f "$test_root/controller.log" ]]; then
    echo "integration controller log:" >&2
    tail -80 "$test_root/controller.log" >&2
  fi
  if [[ $exit_status -ne 0 && ${WEBTOP_TEMPLATE_KEEP_FAILED_ROOT:-0} == 1 ]]; then
    echo "preserved failed integration root: $test_root" >&2
  else
    rm -rf -- "$test_root"
  fi
  return "$exit_status"
}
trap cleanup EXIT

command -v jq >/dev/null
docker image inspect "$test_image" >/dev/null
source_image_id=$(docker image inspect "$test_image" --format '{{.Id}}')
source_repo_digest=$(docker image inspect "$test_image" --format '{{json .RepoDigests}}' | jq -r '.[0] // empty')
if [[ ${WEBTOP_TEMPLATE_REMOVE_SOURCE_IMAGE:-0} == 1 && -z "$source_repo_digest" ]]; then
  docker save --output "$test_root/source-image.tar" "$test_image"
fi
cargo build --workspace >/dev/null

mkdir -p "$test_root/state" "$test_root/environments" "$test_root/snapshots" "$test_root/staging"
WEBTOP_MANAGER_STATE_DIR="$test_root/state" \
WEBTOP_MANAGER_ENVIRONMENT_ROOT="$test_root/environments" \
WEBTOP_MANAGER_HOST_ENVIRONMENT_ROOT="$test_root/environments" \
WEBTOP_MANAGER_SNAPSHOT_ROOT="$test_root/snapshots" \
WEBTOP_MANAGER_STAGING_ROOT="$test_root/staging" \
WEBTOP_MANAGER_SOCKET="$socket_path" \
WEBTOP_MANAGER_WORKER="$project_root/target/debug/webtop-worker" \
"$project_root/target/debug/webtop-controller" >"$test_root/controller.log" 2>&1 &
controller_pid=$!

for _ in $(seq 1 100); do
  [[ -S "$socket_path" ]] && api GET /v1/health >/dev/null && break
  sleep 0.1
done
api GET /v1/health | jq -e '.capabilities | index("template_transfer_v1")' >/dev/null

source_spec=$(jq -n --arg image "$test_image" '{
  name:"integration-source", image:$image,
  identity:{uid:1000,gid:1000,timezone:"Etc/UTC",locale:"en_US.UTF-8"},
  resources:{cpuLimit:null,memoryBytes:null,shmBytes:1073741824},
  display:{width:null,height:null,wayland:null,gpu:"disabled",audio:false,clipboard:false,fileTransfer:false,fileTransferMode:"none"},
  mounts:[], security:{dockerSocket:false,dockerSocketGid:null,privileged:false,seccomp:"default",devices:[]},
  extraEnvironment:{}, publication:{enabled:false,remotePort:null,automaticPort:true}
}')
source=$(api POST /v1/environments "$source_spec")
source_environment_id=$(jq -r .id <<<"$source")
api POST "/v1/environments/$source_environment_id/stop" >/dev/null
printf 'portable-config-round-trip\n' >"$test_root/environments/$source_environment_id/config/integration-proof.txt"

echo "integration: preflight and commit"
preflight=$(api POST "/v1/environments/$source_environment_id/template-preflight")
jq -e '.fileCount >= 1 and .conservativeTotalBytes >= .configOriginalBytes' <<<"$preflight" >/dev/null
create_operation=$(api POST /v1/templates "$(jq -n --arg environment "$source_environment_id" '{environmentId:$environment,name:"integration-template",confirmedSensitiveData:true,confirmedSpaceWarning:true}')")
template_id=$(jq -r .resourceId <<<"$create_operation")
create_result=$(wait_operation "$(jq -r .id <<<"$create_operation")")
jq -e '.logLines | any(contains("snapshot complete")) and any(contains("commit complete"))' <<<"$create_result" >/dev/null

template=$(api GET /v1/templates | jq -c --arg id "$template_id" '.[] | select(.id == $id)')
jq -e '.integrity == "complete" and .platform == "linux/amd64" and .snapshotSizeBytes > 0' <<<"$template" >/dev/null
docker image inspect "com.cue.webtop-manager/template:$template_id" --format '{{json .Config.Entrypoint}} {{json .Config.Volumes}}' | grep -q '/init.*config'

echo "integration: export"
export_operation=$(api POST "/v1/templates/$template_id/exports")
export_result=$(wait_operation "$(jq -r .id <<<"$export_operation")")
jq -e '.logLines | any(contains("image stream complete")) and any(contains("export staged"))' <<<"$export_result" >/dev/null
staging_id=$(jq -r .result.stagingFileId <<<"$export_result")
test -s "$test_root/staging/$staging_id.wtmpl"
password=$(<"$test_root/environments/$source_environment_id/secrets/password")
manifest=$(tar -xOf "$test_root/staging/$staging_id.wtmpl" manifest.json)
source_container=$(docker container ls -aq \
  --filter "label=com.cue.webtop-manager.resource-id=$source_environment_id" | head -1)
[[ -n "$source_container" ]]
inspect_output=$(docker inspect "$source_container")
api_output="$(api GET /v1/environments)$(api GET /v1/templates)$(api GET "/v1/operations/$(jq -r .id <<<"$export_operation")")"
database_strings=$(strings "$test_root/state/controller.sqlite3")
controller_log=$(<"$test_root/controller.log")
for output in "$manifest" "$inspect_output" "$api_output" "$database_strings" "$controller_log"; do
  [[ "$output" != *"$password"* ]]
done

echo "integration: delete local template image and import package"
api DELETE "/v1/environments/$source_environment_id" '{"confirmationName":"integration-source","deleteData":true}' >/dev/null
source_environment_id=""
if [[ ${WEBTOP_TEMPLATE_REMOVE_SOURCE_IMAGE:-0} == 1 ]]; then
  echo "integration: remove source image tag before offline import"
  docker image rm "$test_image" >/dev/null
fi
delete_operation=$(api DELETE "/v1/templates/$template_id" '{"confirmationName":"integration-template"}')
wait_operation "$(jq -r .id <<<"$delete_operation")" >/dev/null
docker image inspect "com.cue.webtop-manager/template:$template_id" >/dev/null 2>&1 && exit 1
if [[ ${WEBTOP_TEMPLATE_REMOVE_SOURCE_IMAGE:-0} == 1 ]]; then
  docker image rm "$source_image_id" >/dev/null 2>&1 || true
  ! docker image inspect "$test_image" >/dev/null 2>&1
  ! docker image inspect "$source_image_id" >/dev/null 2>&1
fi

import_preflight=$(api POST /v1/template-imports/preflight "{\"stagingFileId\":\"$staging_id\"}")
jq -e '.manifest.platform == "linux/amd64" and .untrustedImageWarning' <<<"$import_preflight" >/dev/null
import_operation=$(api POST /v1/template-imports "$(jq -n --arg staging "$staging_id" '{stagingFileId:$staging,name:"integration-imported",confirmedSensitiveData:true,confirmedUntrustedImage:true}')")
imported_template_id=$(jq -r .resourceId <<<"$import_operation")
import_result=$(wait_operation "$(jq -r .id <<<"$import_operation")")
jq -e '.logLines | any(contains("load verified image")) and any(contains("published"))' <<<"$import_result" >/dev/null

echo "integration: restore config into a new environment"
restored_spec=$(jq '.name="integration-restored"' <<<"$source_spec")
restore_operation=$(api POST "/v1/templates/$imported_template_id/environments" "$(jq -n --argjson spec "$restored_spec" '{spec:$spec}')")
restore_result=$(wait_operation "$(jq -r .id <<<"$restore_operation")")
derived_environment_id=$(jq -r .result.environmentId <<<"$restore_result")
grep -qx 'portable-config-round-trip' "$test_root/environments/$derived_environment_id/config/integration-proof.txt"
[[ "$(docker inspect "com.cue.webtop-manager/template:$imported_template_id" --format '{{.Os}}/{{.Architecture}}')" == "linux/amd64" ]]

api DELETE "/v1/environments/$derived_environment_id" '{"confirmationName":"integration-restored","deleteData":true}' >/dev/null
derived_environment_id=""
if [[ ${WEBTOP_TEMPLATE_REMOVE_SOURCE_IMAGE:-0} == 1 ]]; then
  restore_source_image
fi
delete_imported=$(api DELETE "/v1/templates/$imported_template_id" '{"confirmationName":"integration-imported"}')
wait_operation "$(jq -r .id <<<"$delete_imported")" >/dev/null
imported_template_id=""

echo "template integration round-trip passed"
