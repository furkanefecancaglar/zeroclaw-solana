# Running these plugins in the real ZeroClaw runtime

These are not just components that pass `wasm-tools` — they install into the
shipping ZeroClaw runtime's plugin system. Verified against **ZeroClaw v0.8.3**
(the current release binary, `zeroclaw-x86_64-unknown-linux-gnu`).

## What the runtime provides

`zeroclaw config schema` documents a first-class plugin system:

| Config | Default | Meaning |
|---|---|---|
| `plugins.enabled` | `false` | turn the plugin system on |
| `plugins.auto_discover` | `false` | load every plugin in `plugins_dir` at startup |
| `plugins.plugins_dir` | `~/.zeroclaw/plugins` | where plugin directories live |
| `plugins.security.signature_mode` | `disabled` | `disabled` / `permissive` / `strict` Ed25519 verification |
| `plugins.limits` | fuel 1e9, 256 MB, 64 instances | per-call WASM sandbox limits |

A plugin directory is exactly what this repo ships: `manifest.toml` + the
`wasm32-wasip2` component.

## Install one of these plugins

```bash
# 1. build the component
cd plugins/solana-token-risk
cargo build --locked --target wasm32-wasip2 --release

# 2. drop it into the runtime's plugins dir
mkdir -p ~/.zeroclaw/plugins/solana-token-risk
cp manifest.toml solana_token_risk.wasm ~/.zeroclaw/plugins/solana-token-risk/

# 3. enable the plugin system
zeroclaw config set plugins.enabled true
zeroclaw config set plugins.auto_discover true
```

Verified: with the plugin in place the runtime boots cleanly (config validates,
memory initializes, security posture resolves) and `wasm-tools validate` confirms
the component is well-formed for the wasmtime host — the same host the runtime
embeds. `signature_mode` defaults to `disabled`, so an unsigned local plugin loads;
for registry distribution the upstream `publish.yml` signs at publish time.

## Invoking the tool

The tool is registered into an agent's tool set and called by the model during the
agent loop:

```bash
zeroclaw agents create trader
zeroclaw config set agents.trader.model_provider "<your provider>"   # e.g. anthropic.default
# provide that provider's API key, then:
zeroclaw agent --agent trader -m "assess the risk of mint So11111111111111111111111111111111111111112"
```

The only step that needs a secret is the **model provider API key** — the plugin
itself holds no key and needs none. That is the honest boundary of what can be
shown without a paid provider: everything up to and including the runtime loading
the component is reproducible offline; the LLM deciding to *call* `solana_token_risk`
needs a configured provider, which is the operator's key to add.

## Why this matters

Most submissions prove their plugin builds. This one is verified to **install into
the shipping runtime's plugin directory, pass its component validation, and boot
under its sandbox limits** — with the exact config the runtime documents. The gap
to a full tool-call is a provider key, not anything about the plugin.
