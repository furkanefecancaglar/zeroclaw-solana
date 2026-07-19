# solana-verify — a ZeroClaw tool plugin

Offline Solana verification an AI agent can **trust without a network call**. ZeroClaw's
`tool-plugin` WIT world grants no outbound HTTP, so a shippable-today Solana tool must be
pure compute — which is exactly what verification is. This plugin does the checks an agent
handling Solana data actually needs to be sure of, deterministically and offline.

## Ops (dispatch by an `op` field in the JSON args)

| `op` | does | key fields |
|------|------|-----------|
| `merkle_verify`  | folds a **keccak-256 Merkle proof** to an anchored root | `leaf` (hex32), `root` (hex32), `proof: [{hash, right}]` |
| `ed25519_verify` | verifies a **Solana ed25519 signature** over a message | `pubkey` (base58/hex), `message` (hex), `signature` (hex64) |
| `pubkey_decode`  | base58 Solana pubkey → 32 raw bytes | `pubkey` (base58) |
| `pubkey_encode`  | 32 raw bytes → base58 pubkey | `bytes` (hex32) |

A *valid-but-false* verdict (a forged proof, a bad signature) is a **successful** tool call
that reports `"valid": false` — only malformed input returns `success: false`.

### Examples

```jsonc
// verify a TxODDS-style on-chain settlement Merkle proof
{ "op": "merkle_verify",
  "leaf":  "…32-byte hex…",
  "root":  "…anchored root hex…",
  "proof": [ { "hash": "…", "right": true }, { "hash": "…", "right": false } ] }
// → { "ok": true, "valid": true, "hash": "keccak256", "depth": 2, "root": "…" }

// verify a Solana signature
{ "op": "ed25519_verify",
  "pubkey": "6pW64gN1s2uqjHkn1unFeEjAwJkPGHoppGvS715wyP2J",
  "message": "…hex…", "signature": "…128 hex chars…" }
// → { "ok": true, "valid": true, "pubkey": "6pW64…" }
```

## Why keccak Merkle proofs
The flagship op mirrors a real on-chain settlement primitive: TxODDS anchors score/settlement
roots on Solana and a proof either folds to the anchored root or it does not — no oracle to
trust. This plugin lets a ZeroClaw agent verify such a proof itself, before acting on it.
(Built by the team behind a deployed TxODDS on-chain settlement engine.)

## Build & install

```bash
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release      # emits a WASM *component*
cp target/wasm32-wasip2/release/solana_verify.wasm .
# host-side tests of the exact dispatch the component runs:
cargo test --release
# then, in ZeroClaw:
zeroclaw plugin install solana-verify
```

## Layout
- `src/verify.rs` — the pure verification core (keccak fold, ed25519, base58). No wasm dep;
  host-testable with `cargo test`.
- `src/lib.rs` — `handler` (the JSON dispatch, shared with the tests) + the
  `#[cfg(target_family="wasm")]` `wit-bindgen` shim implementing the `tool` interface.
- `manifest.toml` — `capabilities = ["tool"]`, `permissions = []` (pure compute).

Standalone crate, built for `wasm32-wasip2`; not part of a host workspace.
