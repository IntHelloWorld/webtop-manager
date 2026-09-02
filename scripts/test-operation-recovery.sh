#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_root=$(mktemp -d)
socket_path="$test_root/controller.sock"
controller_pid=""
test_image=${WEBTOP_OPERATION_TEST_IMAGE:-lscr.io/linuxserver/webtop:latest}
test_id="00000000-0000-4000-8000-$(printf '%012d' $((RANDOM * RANDOM % 1000000000000)))"

api() {
  curl --connect-timeout 3 --max-time 30 --fail --silent --show-error \
    --unix-socket "$socket_path" "http://localhost$1"
}

start_controller() {
  WEBTOP_MANAGER_STATE_DIR="$test_root/state" \
  WEBTOP_MANAGER_ENVIRONMENT_ROOT="$test_root/environments" \
  WEBTOP_MANAGER_HOST_ENVIRONMENT_ROOT="$test_root/environments" \
  WEBTOP_MANAGER_SNAPSHOT_ROOT="$test_root/snapshots" \
  WEBTOP_MANAGER_STAGING_ROOT="$test_root/staging" \
  WEBTOP_MANAGER_SOCKET="$socket_path" \
  WEBTOP_MANAGER_WORKER="$project_root/target/debug/webtop-worker" \
  WEBTOP_MANAGER_FRPC_CONTAINER_NAME="webtop-manager-frpc-operation-$$" \
  "$project_root/target/debug/webtop-controller" >"$test_root/controller.log" 2>&1 &
  controller_pid=$!
  for _ in $(seq 1 100); do
    [[ -S "$socket_path" ]] && api /v1/health >/dev/null 2>&1 && return 0
    sleep 0.1
  done
  return 1
}

stop_controller() {
  if [[ -n "$controller_pid" ]]; then
    kill "$controller_pid" 2>/dev/null || true
    wait "$controller_pid" 2>/dev/null || true
    controller_pid=""
  fi
}

cleanup() {
  local exit_status=$?
  stop_controller
  if [[ $exit_status -ne 0 && -f "$test_root/controller.log" ]]; then
    tail -100 "$test_root/controller.log" >&2
  fi
  rm -rf -- "$test_root"
  return "$exit_status"
}
trap cleanup EXIT

for command in cargo curl docker jq; do command -v "$command" >/dev/null; done
if ! command -v sqlite3 >/dev/null && ! command -v python3 >/dev/null; then
  echo "sqlite3 or python3 is required" >&2
  exit 1
fi
docker image inspect "$test_image" >/dev/null
cargo build --package webtop-controller --package webtop-worker >/dev/null
mkdir -p "$test_root/state" "$test_root/environments" "$test_root/snapshots" "$test_root/staging"

start_controller
stop_controller

now=$(date --utc +%Y-%m-%dT%H:%M:%SZ)
request=$(jq -cn --arg id "$test_id" --arg reference "$test_image" \
  '{pullId:$id,reference:$reference}')
if command -v sqlite3 >/dev/null; then
  sqlite3 "$test_root/state/controller.sqlite3" <<SQL
INSERT INTO operations (
  id, kind, phase, progress_percent, cancellable, resource_id, error_code,
  error_params_json, result_json, log_json, created_at, updated_at, request_json
) VALUES (
  '$test_id', '"pull_image"', '"running"', 25, 1, NULL, NULL,
  NULL, NULL, '[]', '$now', '$now', '$request'
);
SQL
else
  TEST_DATABASE="$test_root/state/controller.sqlite3" TEST_OPERATION_ID="$test_id" \
    TEST_OPERATION_REQUEST="$request" TEST_OPERATION_NOW="$now" python3 <<'PY'
import os
import sqlite3

connection = sqlite3.connect(os.environ["TEST_DATABASE"])
connection.execute(
    """INSERT INTO operations (
      id, kind, phase, progress_percent, cancellable, resource_id, error_code,
      error_params_json, result_json, log_json, created_at, updated_at, request_json
    ) VALUES (?, '\"pull_image\"', '\"running\"', 25, 1, NULL, NULL,
      NULL, NULL, '[]', ?, ?, ?)""",
    (
        os.environ["TEST_OPERATION_ID"],
        os.environ["TEST_OPERATION_NOW"],
        os.environ["TEST_OPERATION_NOW"],
        os.environ["TEST_OPERATION_REQUEST"],
    ),
)
connection.commit()
PY
fi

start_controller
for _ in $(seq 1 100); do
  operation=$(api "/v1/operations/$test_id")
  phase=$(jq -r .phase <<<"$operation")
  [[ "$phase" == succeeded ]] && break
  [[ "$phase" == failed || "$phase" == retryable ]] && {
    jq . <<<"$operation" >&2
    exit 1
  }
  sleep 0.1
done

jq -e --arg reference "$test_image" '
  .phase == "succeeded"
  and .result.reference == $reference
  and (.logLines | any(contains("reattached interrupted image pull")))
' <<<"$operation" >/dev/null
if command -v sqlite3 >/dev/null; then
  schema_version=$(sqlite3 "$test_root/state/controller.sqlite3" 'PRAGMA user_version;')
else
  schema_version=$(TEST_DATABASE="$test_root/state/controller.sqlite3" python3 <<'PY'
import os
import sqlite3

print(sqlite3.connect(os.environ["TEST_DATABASE"]).execute("PRAGMA user_version").fetchone()[0])
PY
)
fi
[[ "$schema_version" == 2 ]]

echo "durable image-pull controller reattachment passed"
