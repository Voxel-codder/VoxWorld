#!/usr/bin/env bash
set -euo pipefail

export VELOREN_GIT_VERSION="${VELOREN_GIT_VERSION:-/0/0}"

rustup target add wasm32-unknown-unknown

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  cargo install wasm-bindgen-cli --version 0.2.106 --locked
fi

cargo build --release -p voxworld-server-cli -p voxworld-web-gateway --locked
cargo build --release -p voxworld-web-client --target wasm32-unknown-unknown --locked

wasm-bindgen \
  --target web \
  --out-dir web-client/web/pkg \
  target/wasm32-unknown-unknown/release/voxworld_web_client.wasm
