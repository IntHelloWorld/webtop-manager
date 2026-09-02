# Architecture

[简体中文](architecture.zh-CN.md)

## Process boundary

```text
React WebView
  | fixed Tauri commands and redacted events
Tauri bootstrap (unprivileged desktop process)
  | 0600 Unix Socket, versioned /v1 API
Controller container (restart=unless-stopped)
  | Docker API over /var/run/docker.sock
Webtop containers / shared frpc / isolated workers
  | desired state and checkpoints
SQLite WAL + application-owned absolute data paths
```

The WebView cannot submit commands, arbitrary Docker JSON, container names,
host paths for destructive operations, or shell fragments. Tauri validates the
small command surface and forwards typed contracts. The controller repeats
validation because the Unix Socket is a trust boundary.

## Ownership and reconciliation

The controller recognizes only resources carrying all required
`com.cue.webtop-manager.*` labels. It never treats image ancestry, container
name prefixes, Compose labels, or `/config` contents as proof of ownership.

The controller sees environment storage at `/data/environments`, but Docker bind
sources must be host paths. The desktop therefore passes its resolved Tauri
application-data environment root through
`WEBTOP_MANAGER_HOST_ENVIRONMENT_ROOT`; the controller uses the internal path
for filesystem and deletion checks and the host path only when creating managed
Webtop containers.

SQLite records desired state and absolute paths. Docker labels provide the
external identity needed for reconciliation and orphan reporting.

Templates remain independently portable without flattening. A stopped managed
environment is committed as the multi-layer image
`com.cue.webtop-manager/template:<uuid>` and `/config` is snapshotted separately.
Docker content-addressed layers may be shared locally, but the template tag does
not depend on retaining the official source tag. Export uses Docker save/load
semantics so image configuration and every required parent layer travel in the
package. External lineage in an imported manifest is informational; only a
locally created child template receives a local parent foreign key.

Long template jobs are rows in SQLite. Requests return an operation UUID and the
UI polls `GET /v1/operations/{id}`. Closing Tauri does not terminate the
controller task. If the controller itself restarts, non-terminal rows become
`retryable` and controller-owned partial files are removed before serving.
Template import/export additionally accept `DELETE /v1/operations/{id}`. A stop
request moves the operation into rollback; `cancelled` is persisted only after
intermediate files, image tags, and unpublished snapshots have been removed.

## Stable API surface

The first API namespace is `/v1`. Errors contain a stable error code and safe
string parameters only. Internal Docker, SQLite, filesystem and command output
is logged locally after redaction and never copied to frontend events.

Implemented routes in the first code milestone:

- `GET /v1/health`
- `GET|POST /v1/environments`
- `POST /v1/environments/{id}/start|stop|restart`
- `DELETE /v1/environments/{id}` with exact-name confirmation
- `GET /v1/images/official` and allowlisted `POST /v1/images/pull`
- `GET|PUT /v1/settings/server` with the FRP token stored outside SQLite
- `POST /v1/settings/server/token/recover`, accepted only when the protected
  local token is missing or invalid; SQLite stores only its SHA-256 fingerprint
  and recovery state
- `GET /v1/frpc` and fixed `start|restart|stop|test` frpc operations
- `GET /v1/frps/setup` for an explicitly requested, secret-bearing setup command
- `GET|POST /v1/templates`, template preflight, restore, source-check, export and
  dependency-aware delete routes
- `POST /v1/template-imports/preflight|/v1/template-imports` with UUID-only staging
- `GET /v1/operations/{id}` for durable progress and typed results
- `DELETE /v1/operations/{id}` for cooperative template import/export stop
