#!/usr/bin/env bash
set -euo pipefail

export RUST_LOG="${RUST_LOG:-info,common::net=info}"
export VELOREN_GIT_VERSION="${VELOREN_GIT_VERSION:-/0/0}"
export VOXWORLD_USERDATA="${VOXWORLD_USERDATA:-/data/userdata}"
export VOXWORLD_MAX_PLAYERS="${VOXWORLD_MAX_PLAYERS:-100}"

mkdir -p "${VOXWORLD_USERDATA}"

./target/release/voxworld-server-cli --non-interactive --no-auth &
server_pid="$!"

cleanup() {
  kill "${server_pid}" >/dev/null 2>&1 || true
  if [[ -n "${gateway_pid:-}" ]]; then
    kill "${gateway_pid}" >/dev/null 2>&1 || true
  fi
}
trap cleanup INT TERM EXIT

for _ in $(seq 1 90); do
  if (echo > /dev/tcp/127.0.0.1/14004) >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

listen_addr="0.0.0.0:${PORT:-8080}"
upstream_addr="${VOXWORLD_UPSTREAM:-127.0.0.1:14004}"
query_addr="${VOXWORLD_QUERY_SERVER:-127.0.0.1:14006}"
static_dir="${VOXWORLD_WEB_STATIC_DIR:-web-client/web}"
web_max_sessions="${VOXWORLD_WEB_MAX_SESSIONS:-${VOXWORLD_MAX_PLAYERS}}"

./target/release/voxworld-web-gateway \
  --listen "${listen_addr}" \
  --upstream "${upstream_addr}" \
  --query-server "${query_addr}" \
  --max-sessions "${web_max_sessions}" \
  --static-dir "${static_dir}" &
gateway_pid="$!"

wait "${gateway_pid}"
