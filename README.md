# zeroclaw-solana — Solana-native plugins for ZeroClaw 🦞

Three WebAssembly **tool plugins** that give a [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw)
agent Solana capabilities — one live, two offline, all **key-free**. Together they let an agent
**assess** a token's risk, **build** a transaction, and **verify** on-chain data from natural
language, without ever holding a private key.

| Plugin | Live? | What it does |
|--------|-------|--------------|
| [`solana-token-risk`](plugins/solana-token-risk) | **live (`http_client`)** | **Assesses**: reads a mint over `wasi:http` and returns deterministic rug/honeypot risk evidence — mint & freeze authority, Token-2022 dangerous extensions (transfer hook, permanent delegate, transfer fee, non-transferable, default-frozen), holder concentration. |
| [`solana-tx-builder`](plugins/solana-tx-builder) | offline | **Constructs**: PDAs, associated token accounts, SystemProgram & SPL-Token transfer instructions — the agent builds, a wallet signs. |
| [`solana-verify`](plugins/solana-verify) | offline | **Verifies**: keccak-256 Merkle proofs (the TxODDS on-chain settlement primitive), ed25519 signatures, base58 pubkeys. |

## The network model (why one plugin is live and two aren't)
ZeroClaw tool plugins get outbound HTTP **only when they declare the `http_client` permission** —
the host links `wasi:http` after validating that grant (see the tool-plugin guide, *"Tools that
call the network"*). So the split is deliberate, by trust surface:
- **`solana-token-risk`** genuinely needs the chain, so it declares `http_client` and reads live
  over `wasi:http` (via `waki`). Read-only: it fetches account state, never sends a transaction.
- **`solana-tx-builder` / `solana-verify`** are pure compute (`permissions = []`) — they need no
  network to build an instruction or fold a proof, so they take zero trust surface.

An agent can therefore *check a token is safe, build the transfer, and verify the result* — with
exactly one plugin holding a network grant and **none holding a key**.

## Why these
`solana-token-risk` is the plugin the sponsor said they'd *"like to exist most of all"* —
built the way the guide prescribes (pure scoring core + thin `waki` fetch), and it discriminates
on real mainnet data: a renounced token (BONK) scores MINIMAL, a token with live authorities
(USDC) is flagged with the exact on-chain evidence. The verify plugin's flagship op mirrors a
real on-chain primitive: TxODDS anchors settlement roots on Solana and a proof either folds to
the anchored root or it does not — no oracle to trust. Built by the team behind a deployed TxODDS
on-chain settlement engine (World Cup hackathon).

## Layout
Mirrors [`zeroclaw-labs/zeroclaw-plugins`](https://github.com/zeroclaw-labs/zeroclaw-plugins):
`wit/v0/*.wit` + `plugins/<name>/` (pure Rust core + `wit-bindgen` shim + `manifest.toml`).

## Build & test (each plugin is standalone)
```bash
rustup target add wasm32-wasip2
for p in solana-token-risk solana-tx-builder solana-verify; do
  ( cd plugins/$p && cargo build --target wasm32-wasip2 --release && cargo test --release )
done
```
Each build emits a WASM **component** exporting `zeroclaw:plugin/tool` — verified with
`wasm-tools component wit`. No `cargo-component` needed; the `wit_bindgen::generate!` macro
handles it.

### Live demo (real mainnet data, one command)
```bash
cd plugins/solana-token-risk && ./demo.sh          # tests + live BONK vs USDC assessment
```

## Status
- ✅ All three build to `wasm32-wasip2` components; exports verified with `wasm-tools`.
  `solana-token-risk` additionally imports `wasi:http/outgoing-handler@0.2.4` (the http grant).
- ✅ Tests: `solana-verify` 88 · `solana-token-risk` 78 · `solana-tx-builder` 56 — **222 total**,
  covering every risk flag and severity boundary, Merkle-fold conformance (known keccak/sha256
  vectors, forged/truncated/reordered proofs), real ed25519 signature verification and its failure
  modes, PDA/ATA derivation properties, exact instruction byte layouts, malformed-input handling,
  and a prompt-injection fail-closed test per plugin.
- ✅ Key-free by construction: no plugin ever takes a private key; only `solana-token-risk` takes
  a (read-only) network grant.
