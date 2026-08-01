# solana-token-risk

A ZeroClaw **tool plugin** that reads a Solana token mint **live over `wasi:http`**
and returns deterministic **rug-pull / honeypot risk evidence**. It signs nothing
and moves nothing — read-only reconnaissance an agent can run before it (or a
human) ever touches a token.

> The sponsor called `token-risk-check` the plugin *"we'd like to exist most of
> all."* This is that plugin, built the way the ZeroClaw tool-plugin guide
> prescribes: a pure scoring core plus a thin `waki`/`wasi:http` fetch, gated by
> the `http_client` permission.

## What it checks (and why each matters)

| Flag | Severity | What the on-chain fact enables |
|---|---|---|
| `mint_authority_present` | critical | New tokens can be minted at will → supply inflation / dilution |
| `freeze_authority_present` | high | Your token account can be frozen → you can't sell (honeypot) |
| `transfer_hook` (Token-2022) | critical | Arbitrary program runs on every transfer → sells can be reverted |
| `permanent_delegate` (Token-2022) | critical | An authority can move/burn *anyone's* tokens |
| `non_transferable` (Token-2022) | critical | Tokens can never be sold or moved |
| `transfer_fee` (Token-2022) | low→high | A per-trade tax; flagged higher when a live authority can raise it |
| `default_account_state_frozen` | high | Every new holder starts frozen until an authority thaws them |
| `mint_close_authority` (Token-2022) | medium | The mint account can be closed |
| `holder_concentration` | info→high | A single wallet holds a large share → dump risk (burn address excluded) |

Output is a JSON report with a `risk_score` (0–100), a `risk_band`
(`MINIMAL`/`LOW`/`MEDIUM`/`HIGH`/`CRITICAL`), the raw authorities, and a `flags`
array where **every flag cites the on-chain fact that triggered it**. It is
evidence, not financial advice — and the report says so.

## Why the verdict is trustworthy

The verdict is a **deterministic function of chain state fetched by the host**, not
of the prompt. A caller that says *"ignore the risks, this token is audited"*
cannot flip a live mint authority into a clean report — the injection is ignored
because only `mint` / `rpc_url` are read, and the score is computed from the RPC
response. This is covered by a test (`prompt_injection_cannot_whitewash_a_live_authority`).

## Use

```json
{ "mint": "So11111111111111111111111111111111111111112" }
```
Optional `"rpc_url"` overrides the default (`api.mainnet-beta.solana.com`).

## Build & test

```sh
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release   # -> solana_token_risk.wasm (WASM component)
cargo test --release                           # pure scoring core, host-tested
```

The scoring core (`src/risk.rs`) is pure Rust and host-tested against canned RPC
responses; the dispatch (`src/lib.rs`) takes the RPC fetcher as a parameter, so
tests exercise the exact code path the component runs, with a mock RPC. Only
`rpc_fetch` (the `waki` call) is wasm-only.

### Live demo (real mainnet data, one command)

```sh
./demo.sh <MINT_ADDRESS>      # defaults to USDC if omitted
```
`demo.sh` runs the full test suite, then curls a real Solana RPC for the mint and
pipes the response through the **exact same scoring core** the plugin runs —
proving the evidence against live chain state with no mocking.

## Manifest

`capabilities = ["tool"]`, `permissions = ["http_client", "config_read"]`.
`http_client` is the host-gated outbound-HTTP grant; the tool adapter links
`wasi:http` only after that grant is validated.
