# zeroclaw-solana — Solana-native plugins for ZeroClaw 🦞

Five WebAssembly **tool plugins** that give a [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw)
agent Solana capabilities — all **live over `wasi:http`** and all **key-free**. Together they let an
agent **screen** a portfolio, **assess** a token's risk, **build** a transaction, **guard** it
against mainnet, and **verify** on-chain data from natural language — without ever holding a private
key. Every plugin also keeps a pure-compute path, so the deterministic core stays testable and
fail-closed even though each can now read the chain.

| Plugin | Network model | What it does |
|--------|---------------|--------------|
| [`solana-token-risk`](plugins/solana-token-risk) | **live · `http_client`** | **Assesses a mint**: reads it over `wasi:http` and returns deterministic rug/honeypot risk evidence — mint & freeze authority, Token-2022 dangerous extensions (transfer hook, permanent delegate, transfer fee, non-transferable, default-frozen), mutable metadata, and holder concentration that separates off-curve LP vaults from whale wallets. |
| [`solana-wallet-risk`](plugins/solana-wallet-risk) | **live · `http_client`** | **Assesses a portfolio**: scans a wallet's SPL *and* Token-2022 holdings and reports which positions can be frozen, diluted, seized, blocked or taxed — with breadth-weighted wallet-level scoring. |
| [`solana-tx-guard`](plugins/solana-tx-guard) | **live · `http_client`** | **Guards a transaction before it is signed**: statically decodes it across System, SPL-Token/Token-2022, **BPF Upgradeable Loader (program-upgrade hijack), Stake and Vote** programs and flags dangerous instructions (SetAuthority/Upgrade, delegate Approve, FreezeAccount, CloseAccount, stake/vote Authorize, owner Assign), then simulates it live against mainnet and computes the **real balance effect** — it reports exactly how many lamports the fee payer would lose and escalates to DANGEROUS on a genuine drain even when the static decode looks benign. |
| [`solana-tx-builder`](plugins/solana-tx-builder) | **live · `http_client`** | **Constructs**: PDAs, associated token accounts, SystemProgram & SPL-Token transfer instructions. Its live `prepare_transfer` op reads a recent blockhash and checks the recipient exists on chain, so the transfer is **broadcast-ready and typo-safe** — the agent builds, a wallet signs; it never signs or sends. |
| [`solana-verify`](plugins/solana-verify) | **live · `http_client`** | **Verifies**: keccak-256 Merkle proofs (the TxODDS on-chain settlement primitive), ed25519 signatures, base58 pubkeys. Its live `merkle_verify_onchain` op reads the anchored root **straight from chain**, and `merkle_verify_batch` folds **many settlement claims against one root in a single call** (one RPC read for the whole batch) — GREEN only if every claim folds. |

## The network model (live, but least-privilege)
ZeroClaw tool plugins get outbound HTTP **only when they declare the `http_client` permission** —
the host links `wasi:http` after validating that grant (see the tool-plugin guide, *"Tools that
call the network"*). All five declare `http_client` and read the chain **read-only** (via `waki`):
they fetch account state, a recent blockhash, or an anchored root — they **never send a
transaction** and **never hold a key**. The RPC fetch is injected as a parameter into a pure
handler, so the deterministic core (`cargo test`, ~300 cases) runs entirely on the host with mock
RPC, and the exact same code path executes live in the wasm component. Each plugin also keeps its
pure-compute ops (Merkle folds, instruction encoding) that need no network at all — so the tools are
live where the chain adds trust, and fail-closed everywhere else.

An agent can therefore *screen a wallet, check a token is safe, build the transfer, guard it against the chain, and verify the result* — with the network grant confined to the two read-only scanners and **none holding a key**.

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

## Build & test

**One command** — builds all five wasm components, runs all 317 tests, prints the
http-import / tool-export proof per plugin:
```bash
./setup.sh            # or ./setup.sh --quick to skip the wasm builds and just run tests
```

Or drive each plugin standalone:
```bash
rustup target add wasm32-wasip2
for p in solana-token-risk solana-wallet-risk solana-tx-guard solana-tx-builder solana-verify; do
  ( cd plugins/$p && cargo build --target wasm32-wasip2 --release && cargo test --release )
done
```
Each build emits a WASM **component** exporting `zeroclaw:plugin/tool` — verified with
`wasm-tools component wit`. No `cargo-component` needed; the `wit_bindgen::generate!` macro
handles it.

### Composition — the five tools chained (real mainnet, one command)
```bash
./compose-demo.sh          # screen a wallet -> assess its riskiest mint -> build an unsigned exit -> guard it -> verify
```
`solana-wallet-risk` screens a wallet and surfaces the riskiest holding;
`solana-token-risk` deep-dives that exact mint; `solana-tx-builder` constructs the **unsigned** exit transfer;
`solana-tx-guard` decodes and simulates it live before anyone signs; `solana-verify`
is the deterministic trust anchor. Each tool's output feeds the next — the
system-level story, not five isolated tools.

### Running in the real ZeroClaw runtime
These install into the shipping runtime's plugin system (`plugins_dir`,
`signature_mode`, WASM sandbox limits) — verified against ZeroClaw v0.8.3. See
[`RUNTIME.md`](RUNTIME.md) for the exact `zeroclaw config` steps.

### Live demo (real mainnet data, one command)
```bash
cd plugins/solana-token-risk && ./demo.sh          # tests + live BONK vs USDC assessment
```

## Status
- ✅ All five build to `wasm32-wasip2` components; exports verified with `wasm-tools`.
  All five import `wasi:http/outgoing-handler@0.2.4` (the read-only http grant).
- ✅ Tests: `solana-verify` 96 · `solana-token-risk` 79 · `solana-tx-builder` 59 · `solana-wallet-risk` 48 · `solana-tx-guard` 35 — **317 total**,
  covering every risk flag and severity boundary, Merkle-fold conformance (known keccak/sha256
  vectors, forged/truncated/reordered proofs) plus reading the anchored root live from chain, real
  ed25519 signature verification and its failure modes, PDA/ATA derivation properties, exact
  instruction byte layouts, a live blockhash + recipient-existence check, malformed-input handling,
  and a prompt-injection fail-closed test per plugin (including the live ops).
- ✅ Key-free by construction: **no plugin ever takes a private key**; every network grant is
  strictly read-only (account state, blockhash, anchored root) — no plugin can sign or send.
- ✅ **One verdict vocabulary across all five**: every tool returns a top-level
  `agent_verdict` of `RED` (act / do not proceed), `AMBER` (review first) or `GREEN`, plus a
  one-line `reason` — so an agent gets the same actionable shape from every plugin.
