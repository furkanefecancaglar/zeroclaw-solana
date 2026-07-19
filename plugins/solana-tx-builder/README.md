# solana-tx-builder — a ZeroClaw tool plugin

The companion to `solana-verify`: where that **verifies**, this **constructs**. Offline,
pure-compute construction of Solana instructions and addresses an agent can build with no
network egress — a human or wallet signs and sends the result. Nothing here can move funds;
it only produces the bytes to sign.

## Ops (dispatch by an `op` field)

| `op` | does | key fields |
|------|------|-----------|
| `derive_pda`      | `find_program_address(seeds, program)` → address + bump | `program` (b58), `seeds: ["utf8:..","hex:.."]` |
| `derive_ata`      | associated token account for (owner, mint) | `owner` (b58), `mint` (b58) |
| `system_transfer` | a SystemProgram SOL transfer instruction | `from`, `to` (b58), `lamports` |
| `spl_transfer`    | an SPL-Token transfer instruction | `source`, `dest`, `authority` (b58), `amount` |

Instructions come back as `{ program_id, accounts:[{pubkey,is_signer,is_writable}], data_base64, data_hex }`.

### Example
```jsonc
{ "op": "system_transfer", "from": "…b58…", "to": "…b58…", "lamports": 1000000 }
// → { "ok": true, "instruction": { "program_id": "111…", "accounts": [...],
//      "data_base64": "AgAAAEBCDwAAAAAA", "data_hex": "020000004042 0f0000000000" } }
```

## Build & install
```bash
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release   # emits a WASM component
cargo test --release                            # host-side tests of the exact dispatch
```
Standalone crate, built for `wasm32-wasip2`. `capabilities=["tool"]`, `permissions=[]` (pure compute).
