#!/usr/bin/env bash
set -euo pipefail

export RUST_LOG="${RUST_LOG:-info,common::net=info}"
export VELOREN_GIT_VERSION="${VELOREN_GIT_VERSION:-/0/0}"
export VOXWORLD_USERDATA="${VOXWORLD_USERDATA:-/data/userdata}"
export VOXWORLD_MAX_PLAYERS="${VOXWORLD_MAX_PLAYERS:-100}"
server_startup_timeout="${VOXWORLD_SERVER_STARTUP_TIMEOUT:-90}"

mkdir -p "${VOXWORLD_USERDATA}"

listen_addr="0.0.0.0:${PORT:-8080}"
upstream_addr="${VOXWORLD_UPSTREAM:-127.0.0.1:14004}"
upstream_host="${upstream_addr%:*}"
upstream_port="${upstream_addr##*:}"
query_addr="${VOXWORLD_QUERY_SERVER:-127.0.0.1:14006}"
static_dir="${VOXWORLD_WEB_STATIC_DIR:-voxygen-web/web}"
web_max_sessions="${VOXWORLD_WEB_MAX_SESSIONS:-${VOXWORLD_MAX_PLAYERS}}"

./target/release/voxworld-server-cli --non-interactive --no-auth &
server_pid="$!"

cleanup() {
  kill "${server_pid}" >/dev/null 2>&1 || true
  if [[ -n "${gateway_pid:-}" ]]; then
    kill "${gateway_pid}" >/dev/null 2>&1 || true
  fi
}
trap cleanup INT TERM EXIT

server_ready="0"
for _ in $(seq 1 "${server_startup_timeout}"); do
  if ! kill -0 "${server_pid}" >/dev/null 2>&1; then
    set +e
    wait "${server_pid}"
    server_status="$?"
    set -e
    echo "Vox World server exited before accepting game connections (status ${server_status})." >&2
    if [[ "${server_status}" -eq 0 ]]; then
      exit 1
    fi
    exit "${server_status}"
  fi

  if (echo > "/dev/tcp/${upstream_host}/${upstream_port}") >/dev/null 2>&1; then
    server_ready="1"
    break
  fi
  sleep 1
done

if [[ "${server_ready}" != "1" ]]; then
  echo "Vox World server did not accept game connections within ${server_startup_timeout}s." >&2
  exit 1
fi

./target/release/voxworld-web-gateway \
  --listen "${listen_addr}" \
  --upstream "${upstream_addr}" \
  --query-server "${query_addr}" \
  --max-sessions "${web_max_sessions}" \
  --static-dir "${static_dir}" &
gateway_pid="$!"

set +e
wait -n "${server_pid}" "${gateway_pid}"
service_status="$?"
set -e

echo "A Vox World service process exited; stopping Railway service (status ${service_status})." >&2
if [[ "${service_status}" -eq 0 ]]; then
  exit 1
fi
exit "${service_status}"
