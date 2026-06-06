#!/usr/bin/env bash
set -euo pipefail

export VELOREN_GIT_VERSION="${VELOREN_GIT_VERSION:-/0/0}"
wasm_bindgen_version="${WASM_BINDGEN_VERSION:-0.2.106}"

rustup target add wasm32-unknown-unknown

installed_wasm_bindgen_version="$(wasm-bindgen --version 2>/dev/null || true)"
if [[ "${installed_wasm_bindgen_version}" != "wasm-bindgen ${wasm_bindgen_version}" ]]; then
  cargo install wasm-bindgen-cli --version "${wasm_bindgen_version}" --locked --force
fi

cargo build --release -p voxworld-server-cli -p voxworld-web-gateway --locked
cargo build --release -p voxworld-web-client --target wasm32-unknown-unknown --locked

rm -rf web-client/web/pkg
wasm-bindgen \
  --target web \
  --out-dir web-client/web/pkg \
  target/wasm32-unknown-unknown/release/voxworld_web_client.wasm

test -s web-client/web/pkg/voxworld_web_client.js
test -s web-client/web/pkg/voxworld_web_client_bg.wasm
