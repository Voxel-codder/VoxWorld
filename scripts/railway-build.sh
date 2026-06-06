#!/usr/bin/env bash
set -euo pipefail

export VELOREN_GIT_VERSION="${VELOREN_GIT_VERSION:-/0/0}"
wasm_bindgen_version="${WASM_BINDGEN_VERSION:-0.2.106}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command '$1' was not found in PATH." >&2
    exit 127
  fi
}

require_command cargo

if command -v rustup >/dev/null 2>&1; then
  rustup target add wasm32-unknown-unknown
else
  echo "rustup was not found; assuming the wasm32-unknown-unknown target is already installed." >&2
fi

installed_wasm_bindgen_version="$(wasm-bindgen --version 2>/dev/null || true)"
if [[ "${installed_wasm_bindgen_version}" != "wasm-bindgen ${wasm_bindgen_version}" ]]; then
  cargo install wasm-bindgen-cli --version "${wasm_bindgen_version}" --locked --force
fi

cargo build --release -p voxworld-server-cli -p voxworld-web-gateway --locked
cargo build --release -p voxworld-voxygen-web --target wasm32-unknown-unknown --locked

rm -rf voxygen-web/web/pkg
wasm-bindgen \
  --target web \
  --out-dir voxygen-web/web/pkg \
  target/wasm32-unknown-unknown/release/voxworld_voxygen_web.wasm

test -s voxygen-web/web/pkg/voxworld_voxygen_web.js
test -s voxygen-web/web/pkg/voxworld_voxygen_web_bg.wasm
