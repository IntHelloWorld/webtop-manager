#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_root=$(mktemp -d)
socket_path="$test_root/controller.sock"
test_id="integration-$$"
frps_container="webtop-manager-frps-$test_id"
external_frpc_container="webtop-manager-external-frpc-$test_id"
managed_frpc_container="webtop-manager-frpc-$test_id"
controller_pid=""
environment_id=""
test_image=${WEBTOP_FRP_TEST_IMAGE:-lscr.io/linuxserver/webtop:latest}
frpc_image='ghcr.io/fatedier/frpc:v0.70.1@sha256:e6483f2a916de67281597ba8fd03dc25d4f6fbd7ed0eafa042b2a5e4dcb5ee22'
frps_image='ghcr.io/fatedier/frps:v0.70.1@sha256:dab4febe235a24ddda5c20b1971ce34a31dc9f33983db3b126d278b932650408'

api() {
  local method=$1
  local path=$2
  local body=${3-}
  if [[ -n "$body" ]]; then
    curl --connect-timeout 3 --max-time 120 --fail --silent --show-error \
      --unix-socket "$socket_path" -X "$method" -H 'content-type: application/json' \
      --data "$body" "http://localhost$path"
  else
    curl --connect-timeout 3 --max-time 120 --fail --silent --show-error \
      --unix-socket "$socket_path" -X "$method" "http://localhost$path"
  fi
}

port_is_free() {
  local port=$1
  ! ss -H -ltn | awk '{print $4}' | grep -Eq "(^|[:.])${port}$"
}

find_free_range() {
  local width=$1
  local first=$2
  local last=$3
  local start port offset available
  for start in $(seq "$first" 10 "$last"); do
    available=1
    for offset in $(seq 0 $((width - 1))); do
      port=$((start + offset))
      if ! port_is_free "$port"; then available=0; break; fi
    done
    if [[ $available == 1 ]]; then echo "$start"; return 0; fi
  done
  return 1
}

wait_https() {
  local port=$1
  for _ in $(seq 1 180); do
    if curl --insecure --location --connect-timeout 2 --max-time 4 --silent \
      --output /dev/null "https://127.0.0.1:$port/"; then
      return 0
    fi
    sleep 1
  done
  echo "HTTPS endpoint did not become ready on port $port" >&2
  return 1
}

cleanup() {
  local exit_status=$?
  set +e
  if [[ -n "$environment_id" ]]; then
    api DELETE "/v1/environments/$environment_id" \
      '{"confirmationName":"integration-frp","deleteData":true}' >/dev/null
  fi
  docker container rm -f "$managed_frpc_container" "$external_frpc_container" \
    "$frps_container" >/dev/null 2>&1
  if [[ -n "$controller_pid" ]]; then
    kill "$controller_pid" 2>/dev/null
    wait "$controller_pid" 2>/dev/null
  fi
  if [[ -n "$environment_id" ]]; then
    container=$(docker container ls -aq \
      --filter "label=com.cue.webtop-manager.resource-id=$environment_id" | head -1)
    [[ -n "$container" ]] && docker container rm -f "$container" >/dev/null
  fi
  if [[ $exit_status -ne 0 ]]; then
    docker logs "$managed_frpc_container" 2>&1 | tail -80 >&2
    docker logs "$external_frpc_container" 2>&1 | tail -80 >&2
    docker logs "$frps_container" 2>&1 | tail -80 >&2
    [[ -f "$test_root/controller.log" ]] && tail -100 "$test_root/controller.log" >&2
  fi
  rm -rf -- "$test_root"
  return "$exit_status"
}
trap cleanup EXIT

for command in cargo curl docker jq openssl ss; do command -v "$command" >/dev/null; done
docker image inspect "$test_image" >/dev/null
docker image inspect "$frpc_image" >/dev/null
docker image inspect "$frps_image" >/dev/null

bind_port=$(find_free_range 1 44000 44900)
remote_port_start=$(find_free_range 6 45000 49000)
remote_port_end=$((remote_port_start + 5))

cargo build --workspace >/dev/null
mkdir -p "$test_root/state" "$test_root/environments" "$test_root/snapshots" \
  "$test_root/staging" "$test_root/frps" "$test_root/external-frpc"

WEBTOP_MANAGER_STATE_DIR="$test_root/state" \
WEBTOP_MANAGER_ENVIRONMENT_ROOT="$test_root/environments" \
WEBTOP_MANAGER_HOST_ENVIRONMENT_ROOT="$test_root/environments" \
WEBTOP_MANAGER_SNAPSHOT_ROOT="$test_root/snapshots" \
WEBTOP_MANAGER_STAGING_ROOT="$test_root/staging" \
WEBTOP_MANAGER_SOCKET="$socket_path" \
WEBTOP_MANAGER_WORKER="$project_root/target/debug/webtop-worker" \
WEBTOP_MANAGER_FRPC_CONTAINER_NAME="$managed_frpc_container" \
"$project_root/target/debug/webtop-controller" >"$test_root/controller.log" 2>&1 &
controller_pid=$!

for _ in $(seq 1 100); do
  [[ -S "$socket_path" ]] && api GET /v1/health >/dev/null && break
  sleep 0.1
done
api GET /v1/health | jq -e '.capabilities | index("durable_image_pull_v1")' >/dev/null

settings=$(jq -n \
  --arg host '127.0.0.1' \
  --arg publicIp '127.0.0.1' \
  --arg image "$frpc_image" \
  --argjson bind "$bind_port" \
  --argjson start "$remote_port_start" \
  --argjson end "$remote_port_end" \
  '{settings:{frpsHost:$host,frpsPort:$bind,publicIp:$publicIp,remotePortStart:$start,remotePortEnd:$end,tokenConfigured:false,frpcImage:$image}}')
api PUT /v1/settings/server "$settings" >/dev/null
token=$(<"$test_root/state/secrets/frp-token")
[[ ${#token} -ge 40 ]]

cat >"$test_root/frps/frps.toml" <<EOF
bindPort = $bind_port
auth.method = "token"
auth.tokenSource.type = "file"
auth.tokenSource.file.path = "/etc/frp/frp-token"
allowPorts = [{ start = $remote_port_start, end = $remote_port_end }]
EOF
printf '%s' "$token" >"$test_root/frps/frp-token"
chmod 600 "$test_root/frps/frps.toml" "$test_root/frps/frp-token"
docker run -d --name "$frps_container" --network host \
  -v "$test_root/frps/frps.toml:/etc/frp/frps.toml:ro" \
  -v "$test_root/frps/frp-token:/etc/frp/frp-token:ro" \
  "$frps_image" -c /etc/frp/frps.toml >/dev/null

for _ in $(seq 1 80); do
  if docker logs "$frps_container" 2>&1 | grep -q 'frps started successfully'; then break; fi
  sleep 0.25
done

cat >"$test_root/external-frpc/frpc.toml" <<EOF
serverAddr = "127.0.0.1"
serverPort = $bind_port
loginFailExit = true
auth.method = "token"
auth.tokenSource.type = "file"
auth.tokenSource.file.path = "/run/frp-token"

[[proxies]]
name = "external-port-holder-$test_id"
type = "tcp"
localIP = "127.0.0.1"
localPort = 9
remotePort = $remote_port_start
EOF
printf '%s' "$token" >"$test_root/external-frpc/frp-token"
chmod 600 "$test_root/external-frpc/frpc.toml" "$test_root/external-frpc/frp-token"
docker run -d --name "$external_frpc_container" --network host \
  -v "$test_root/external-frpc/frpc.toml:/etc/frp/frpc.toml:ro" \
  -v "$test_root/external-frpc/frp-token:/run/frp-token:ro" \
  "$frpc_image" -c /etc/frp/frpc.toml >/dev/null
for _ in $(seq 1 80); do
  if docker logs "$external_frpc_container" 2>&1 | grep -q 'start proxy success'; then break; fi
  sleep 0.25
done
docker logs "$external_frpc_container" 2>&1 | grep -q 'start proxy success'

spec=$(jq -n --arg image "$test_image" '{
  name:"integration-frp", image:$image,
  identity:{uid:1000,gid:1000,timezone:"Etc/UTC",locale:"en_US.UTF-8"},
  resources:{cpuLimit:null,memoryBytes:null,shmBytes:1073741824},
  display:{width:null,height:null,wayland:null,gpu:"disabled",audio:false,clipboard:false,fileTransfer:false,fileTransferMode:"none"},
  mounts:[], security:{dockerSocket:false,dockerSocketGid:null,privileged:false,seccomp:"default",devices:[]},
  extraEnvironment:{}, publication:{enabled:false,remotePort:null,automaticPort:true}
}')
environment=$(api POST /v1/environments "$spec")
environment_id=$(jq -r .id <<<"$environment")
local_port=$(jq -r .localPort <<<"$environment")
wait_https "$local_port"

api POST /v1/frpc/start >/dev/null
published=$(api POST "/v1/environments/$environment_id/publish")
published_port=$(jq -r .spec.publication.remotePort <<<"$published")
[[ "$published_port" == "$((remote_port_start + 1))" ]]
wait_https "$published_port"
openssl s_client -connect "127.0.0.1:$published_port" -servername localhost </dev/null \
  2>/dev/null | openssl x509 -noout -subject >/dev/null

password=$(<"$test_root/environments/$environment_id/secrets/password")
environment_container=$(docker container ls -aq \
  --filter "label=com.cue.webtop-manager.resource-id=$environment_id" | head -1)
[[ -n "$environment_container" ]]
inspect_output=$(docker inspect "$managed_frpc_container" "$environment_container")
api_output="$(api GET /v1/environments)$(api GET /v1/settings/server)$(api GET /v1/frpc)"
database_strings=$(strings "$test_root/state/controller.sqlite3")
controller_log=$(<"$test_root/controller.log")
for secret in "$token" "$password"; do
  [[ "$inspect_output" != *"$secret"* ]]
  [[ "$api_output" != *"$secret"* ]]
  [[ "$database_strings" != *"$secret"* ]]
  [[ "$controller_log" != *"$secret"* ]]
done

echo "FRP port-race, public TLS, and secret-leak integration passed"
