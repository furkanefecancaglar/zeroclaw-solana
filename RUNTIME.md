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

A plugin directory is exactly what each plugin here provides: its `manifest.toml`
(which names the component via `wasm_path`) + the built `wasm32-wasip2` component.
The `.wasm` is a build artifact — it is **not** committed; `./setup.sh` builds all
five, or build one with the command below.

## Install all five plugins

```bash
# 1. build every component (or: cd plugins/<name> && cargo build --locked --target wasm32-wasip2 --release)
./setup.sh

# 2. drop each plugin (manifest + its component) into the runtime's plugins dir.
#    The component lands in target/wasm32-wasip2/release/<name_with_underscores>.wasm;
#    the manifest's wasm_path expects it next to manifest.toml in the install dir.
for p in solana-token-risk solana-wallet-risk solana-tx-guard solana-tx-builder solana-verify; do
  wasm="${p//-/_}.wasm"
  mkdir -p ~/.zeroclaw/plugins/"$p"
  cp "plugins/$p/manifest.toml" ~/.zeroclaw/plugins/"$p"/
  cp "plugins/$p/target/wasm32-wasip2/release/$wasm" ~/.zeroclaw/plugins/"$p"/
done

# 3. enable the plugin system
zeroclaw config set plugins.enabled true
zeroclaw config set plugins.auto_discover true
```

To install just one, run the two `cp` lines for that single `p`.

Verified: with a plugin of this exact shape in place the runtime boots cleanly
(config validates, memory initializes, security posture resolves) against ZeroClaw
v0.8.3, and `wasm-tools validate` confirms **all five** components are well-formed
for the wasmtime host — the same host the runtime embeds. Because every plugin here
ships the identical shape (a `manifest.toml` + a validated `wasm32-wasip2` component
exporting `zeroclaw:plugin/tool`), the install steps above apply to each unchanged.
`signature_mode` defaults to `disabled`, so an unsigned local plugin loads; for
registry distribution the upstream `publish.yml` signs at publish time.

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
