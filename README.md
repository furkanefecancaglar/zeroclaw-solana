# zeroclaw-solana — Solana-native plugins for ZeroClaw 🦞

Two WebAssembly **tool plugins** that give a [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw)
agent Solana capabilities. ZeroClaw's `tool-plugin` WIT world grants **no outbound network**,
so a shippable-today Solana tool must be pure compute — which is exactly what verification and
transaction construction are. These two cover the trust-critical half of working with Solana
data, offline and deterministically.

| Plugin | What it does |
|--------|--------------|
| [`solana-verify`](plugins/solana-verify) | **Verifies**: keccak-256 Merkle proofs (the TxODDS on-chain settlement primitive), ed25519 signatures, base58 pubkeys. |
| [`solana-tx-builder`](plugins/solana-tx-builder) | **Constructs**: PDAs, associated token accounts, SystemProgram & SPL-Token transfer instructions — the agent builds, a wallet signs. |

Together: *"an agent can build and verify Solana transactions from natural language, without
ever touching a key or the network."*

## Why these
The flagship op mirrors a real on-chain primitive: TxODDS anchors score/settlement roots on
Solana and a proof either folds to the anchored root or it does not — no oracle to trust.
Built by the team behind a deployed TxODDS on-chain settlement engine (World Cup hackathon).

## Layout
Mirrors [`zeroclaw-labs/zeroclaw-plugins`](https://github.com/zeroclaw-labs/zeroclaw-plugins):
`wit/v0/*.wit` + `plugins/<name>/` (pure Rust core + `wit-bindgen` shim + `manifest.toml`).

## Build (each plugin is standalone)
```bash
rustup target add wasm32-wasip2
cd plugins/solana-verify   && cargo build --target wasm32-wasip2 --release && cargo test --release
cd plugins/solana-tx-builder && cargo build --target wasm32-wasip2 --release && cargo test --release
```
Each build emits a WASM **component** exporting `zeroclaw:plugin/tool` — verified with
`wasm-tools component wit`. No `cargo-component` needed; the `wit_bindgen::generate!` macro
handles it. 13 host tests across the two plugins.

## Status
- ✅ Both build to `wasm32-wasip2` components; exports verified.
- ✅ `solana-verify`: 6 tests · `solana-tx-builder`: 7 tests.
- Pure compute (`permissions = []`) — no network, matching the current WIT `tool-plugin` world.
