#!/usr/bin/env bash
# One command to build and verify the whole suite from a clean checkout.
#   ./setup.sh          build all 5 wasm components + run every test
#   ./setup.sh --quick  skip the wasm builds, just run the host tests
#
# Needs: rustup + cargo. Installs the wasm32-wasip2 target if missing.
# wasm-tools is optional — used only to print the http-import / tool-export proof.
set -euo pipefail
cd "$(dirname "$0")"

PLUGINS=(solana-token-risk solana-wallet-risk solana-tx-guard solana-tx-builder solana-verify)
QUICK=0; [ "${1:-}" = "--quick" ] && QUICK=1

bold() { printf '\033[1m%s\033[0m\n' "$1"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }

bold "zeroclaw-solana — build & verify (5 plugins)"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found. Install Rust from https://rustup.rs and re-run." >&2
  exit 1
fi

if [ "$QUICK" -eq 0 ]; then
  if ! rustup target list --installed 2>/dev/null | grep -q wasm32-wasip2; then
    echo "installing wasm32-wasip2 target..."
    rustup target add wasm32-wasip2
  fi
  ok "wasm32-wasip2 target ready"
fi

total=0
for p in "${PLUGINS[@]}"; do
  bold "→ $p"
  if [ "$QUICK" -eq 0 ]; then
    ( cd "plugins/$p" && cargo build --target wasm32-wasip2 --release --quiet )
    ok "wasm32-wasip2 component built"
    wasm="plugins/$p/target/wasm32-wasip2/release/${p//-/_}.wasm"
    if command -v wasm-tools >/dev/null 2>&1 && [ -f "$wasm" ]; then
      http=$(wasm-tools component wit "$wasm" 2>/dev/null | grep -c 'wasi:http/outgoing-handler' || true)
      tool=$(wasm-tools component wit "$wasm" 2>/dev/null | grep -c 'export zeroclaw:plugin/tool' || true)
      ok "imports wasi:http=${http}  exports tool=${tool}"
    fi
  fi
  n=$( ( cd "plugins/$p" && cargo test --release --quiet 2>&1 ) | grep -E 'test result' | awk '{s+=$4} END{print s+0}')
  ok "$n tests passed"
  total=$((total + n))
done

bold "DONE — ${#PLUGINS[@]} plugins, ${total} tests passed."
echo "Next: ./compose-demo.sh (the five tools chained on real mainnet data)."
