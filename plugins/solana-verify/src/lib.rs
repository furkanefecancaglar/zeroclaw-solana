//! A ZeroClaw WIT tool plugin: `solana-verify`.
//!
//! Local, pure-compute verification an AI agent can trust without any network egress
//! (the `tool-plugin` WIT world grants no outbound HTTP). One tool, dispatched by an `op`
//! field:
//!   * `merkle_verify`   — fold a keccak-256 Merkle proof to an anchored root
//!                         (the exact TxODDS on-chain settlement primitive).
//!   * `ed25519_verify`  — verify a Solana ed25519 signature over a message.
//!   * `pubkey_decode`   — base58 Solana pubkey → 32 raw bytes (hex).
//!   * `pubkey_encode`   — 32 raw bytes (hex) → base58 pubkey.
//!
//! The pure core lives in [`verify`] with no wasm dependency, so it compiles and tests on
//! the host with a plain `cargo test`; the wasm component reuses the exact same logic.
//!
//! Build:  rustup target add wasm32-wasip2
//!         cargo build --target wasm32-wasip2 --release

pub mod verify;

/// Shared, wasm-independent request handling so the host `cargo test` exercises the exact
/// dispatch the component runs. Input/Output are JSON strings.
pub mod handler {
    use crate::verify::*;
    use base64::Engine as _;
    use serde::Deserialize;
    use serde_json::{json, Value};

    #[derive(Deserialize)]
    struct ProofIn {
        hash: String,
        #[serde(default)]
        right: bool,
    }

    /// Default Solana mainnet RPC when the caller passes no `rpc_url` and config has none.
    pub const DEFAULT_RPC: &str = "https://api.mainnet-beta.solana.com";

    /// Performs one Solana JSON-RPC call: `(url, method, params) -> result`. On the host it
    /// is a mock; in the wasm component it is a `waki` POST over `wasi:http`. Injecting it
    /// keeps every pure op testable and exercises the exact live dispatch under `cargo test`.
    pub type Fetcher<'a> = dyn Fn(&str, &str, Value) -> Result<Value, String> + 'a;

    /// Run one `solana-verify` op. Returns (output_json, ok). `ok` is false only for
    /// malformed input; a *valid-but-false* verdict (e.g. a forged proof) is a successful
    /// tool call that reports `"valid": false`. Only `merkle_verify_onchain` touches the
    /// network; the pure-compute ops ignore `fetch`.
    pub fn run(args: &str, fetch: &Fetcher) -> (String, bool) {
        let v: Value = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return (err(&format!("invalid JSON args: {e}")), false),
        };
        let op = v.get("op").and_then(|o| o.as_str()).unwrap_or("");
        match op {
            "merkle_verify" => merkle(&v),
            "merkle_verify_onchain" => merkle_onchain(&v, fetch),
            "ed25519_verify" => ed25519(&v),
            "pubkey_decode" => pubkey_decode(&v),
            "pubkey_encode" => pubkey_encode(&v),
            "" => (err("missing 'op' (merkle_verify|merkle_verify_onchain|ed25519_verify|pubkey_decode|pubkey_encode)"), false),
            other => (err(&format!("unknown op '{other}'")), false),
        }
    }

    fn err(msg: &str) -> String {
        json!({ "ok": false, "error": msg }).to_string()
    }

    fn merkle(v: &Value) -> (String, bool) {
        let leaf = match field_hex32(v, "leaf") { Ok(x) => x, Err(e) => return (err(&e), false) };
        let root = match field_hex32(v, "root") { Ok(x) => x, Err(e) => return (err(&e), false) };
        let proof_val = v.get("proof").cloned().unwrap_or(json!([]));
        let nodes_in: Vec<ProofIn> = match serde_json::from_value(proof_val) {
            Ok(n) => n,
            Err(e) => return (err(&format!("bad proof array: {e}")), false),
        };
        let mut proof = Vec::with_capacity(nodes_in.len());
        for n in &nodes_in {
            match hex32(&n.hash) {
                Ok(h) => proof.push(ProofNode { hash: h, is_right_sibling: n.right }),
                Err(e) => return (err(&format!("bad proof node hash: {e}")), false),
            }
        }
        let valid = merkle_verify(leaf, &proof, root);
        (json!({
            "ok": true, "op": "merkle_verify", "valid": valid,
            "hash": "keccak256", "depth": proof.len(),
            "root": to_hex(&root),
        }).to_string(), true)
    }

    /// Parse the `proof` array of `{hash, right}` nodes shared by both merkle ops.
    fn parse_proof(v: &Value) -> Result<Vec<ProofNode>, String> {
        let proof_val = v.get("proof").cloned().unwrap_or(json!([]));
        let nodes_in: Vec<ProofIn> =
            serde_json::from_value(proof_val).map_err(|e| format!("bad proof array: {e}"))?;
        let mut proof = Vec::with_capacity(nodes_in.len());
        for n in &nodes_in {
            let h = hex32(&n.hash).map_err(|e| format!("bad proof node hash: {e}"))?;
            proof.push(ProofNode { hash: h, is_right_sibling: n.right });
        }
        Ok(proof)
    }

    /// Live variant of `merkle_verify`: instead of trusting a caller-supplied `root`, read
    /// the anchored root straight from chain. Fetches `getAccountInfo(account, base64)`,
    /// takes the 32 bytes at `offset` (default 0) as the root, and folds the proof against
    /// it. This closes the trust gap — the settlement root comes from the chain, not the
    /// prompt — so a prompt-injected "trust me, it's settled" cannot flip the verdict.
    fn merkle_onchain(v: &Value, fetch: &Fetcher) -> (String, bool) {
        let leaf = match field_hex32(v, "leaf") { Ok(x) => x, Err(e) => return (err(&e), false) };
        let account = match v.get("account").and_then(|x| x.as_str()) {
            Some(a) => a.to_string(),
            None => return (err("missing 'account' (base58 pubkey holding the anchored root)"), false),
        };
        // reject a non-base58 / wrong-length account early with a clear message.
        if b58_32(&account).is_err() {
            return (err("'account' must be a base58 32-byte Solana pubkey"), false);
        }
        let offset = v.get("offset").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let proof = match parse_proof(v) { Ok(p) => p, Err(e) => return (err(&e), false) };
        let rpc = v.get("rpc_url").and_then(|x| x.as_str()).unwrap_or(DEFAULT_RPC);

        let resp = match fetch(rpc, "getAccountInfo", json!([account, {"encoding": "base64"}])) {
            Ok(r) => r,
            Err(e) => return (err(&format!("RPC getAccountInfo failed: {e}")), false),
        };
        let value = &resp["result"]["value"];
        if value.is_null() {
            return (err(&format!("account {account} not found on chain")), false);
        }
        let b64 = match value["data"][0].as_str() {
            Some(s) => s,
            None => return (err("account data missing (expected base64 encoding)"), false),
        };
        let data = match base64::engine::general_purpose::STANDARD.decode(b64) {
            Ok(d) => d,
            Err(e) => return (err(&format!("account data is not valid base64: {e}")), false),
        };
        if data.len() < offset + 32 {
            return (err(&format!(
                "account data too short: need 32 bytes at offset {offset}, have {}",
                data.len()
            )), false);
        }
        let mut root = [0u8; 32];
        root.copy_from_slice(&data[offset..offset + 32]);
        let slot = resp["result"]["context"]["slot"].as_u64();

        let valid = merkle_verify(leaf, &proof, root);
        (json!({
            "ok": true, "op": "merkle_verify_onchain", "valid": valid,
            "hash": "keccak256", "depth": proof.len(),
            "account": account, "offset": offset, "slot": slot,
            "root": to_hex(&root), "source": "on-chain",
        }).to_string(), true)
    }

    fn ed25519(v: &Value) -> (String, bool) {
        let pk = match field_pubkey(v, "pubkey") { Ok(x) => x, Err(e) => return (err(&e), false) };
        let msg = match field_bytes(v, "message") { Ok(x) => x, Err(e) => return (err(&e), false) };
        let sig_bytes = match field_bytes(v, "signature") { Ok(x) => x, Err(e) => return (err(&e), false) };
        let sig: [u8; 64] = match sig_bytes.try_into() {
            Ok(s) => s,
            Err(_) => return (err("signature must be 64 bytes"), false),
        };
        let valid = ed25519_verify(&pk, &msg, &sig);
        (json!({ "ok": true, "op": "ed25519_verify", "valid": valid,
                 "pubkey": b58_encode(&pk) }).to_string(), true)
    }

    fn pubkey_decode(v: &Value) -> (String, bool) {
        let pk = match field_pubkey(v, "pubkey") { Ok(x) => x, Err(e) => return (err(&e), false) };
        (json!({ "ok": true, "op": "pubkey_decode", "bytes_hex": to_hex(&pk) }).to_string(), true)
    }

    fn pubkey_encode(v: &Value) -> (String, bool) {
        let b = match field_hex32(v, "bytes") { Ok(x) => x, Err(e) => return (err(&e), false) };
        (json!({ "ok": true, "op": "pubkey_encode", "pubkey": b58_encode(&b) }).to_string(), true)
    }

    // field extractors: accept hex ("0x.."/"..") or, for pubkeys, base58.
    fn field_hex32(v: &Value, k: &str) -> Result<[u8; 32], String> {
        let s = v.get(k).and_then(|x| x.as_str()).ok_or(format!("missing '{k}' (32-byte hex)"))?;
        hex32(s)
    }
    fn field_bytes(v: &Value, k: &str) -> Result<Vec<u8>, String> {
        let s = v.get(k).and_then(|x| x.as_str()).ok_or(format!("missing '{k}'"))?;
        from_hex(s)
    }
    fn field_pubkey(v: &Value, k: &str) -> Result<[u8; 32], String> {
        let s = v.get(k).and_then(|x| x.as_str()).ok_or(format!("missing '{k}' (base58 pubkey)"))?;
        // try base58 first (Solana pubkeys), then hex
        b58_32(s).or_else(|_| hex32(s))
    }

    pub const SCHEMA: &str = r#"{
      "type": "object",
      "properties": {
        "op": {"type": "string", "enum": ["merkle_verify","merkle_verify_onchain","ed25519_verify","pubkey_decode","pubkey_encode"],
               "description": "Which Solana check to run. merkle_verify_onchain reads the anchored root live from chain; the rest are local no-network."},
        "leaf": {"type": "string", "description": "merkle_verify[_onchain]: 32-byte leaf hash, hex."},
        "root": {"type": "string", "description": "merkle_verify: 32-byte anchored root, hex."},
        "account": {"type": "string", "description": "merkle_verify_onchain: base58 account holding the anchored root on chain."},
        "offset": {"type": "integer", "description": "merkle_verify_onchain: byte offset of the 32-byte root in the account data (default 0)."},
        "rpc_url": {"type": "string", "description": "merkle_verify_onchain: optional Solana RPC endpoint (defaults to mainnet-beta)."},
        "proof": {"type": "array", "description": "merkle_verify[_onchain]: sibling path.",
                  "items": {"type": "object",
                    "properties": {"hash": {"type": "string"}, "right": {"type": "boolean"}},
                    "required": ["hash"]}},
        "pubkey": {"type": "string", "description": "base58 Solana pubkey (or 32-byte hex)."},
        "message": {"type": "string", "description": "ed25519_verify: signed message, hex."},
        "signature": {"type": "string", "description": "ed25519_verify: 64-byte signature, hex."},
        "bytes": {"type": "string", "description": "pubkey_encode: 32 raw bytes, hex."}
      },
      "required": ["op"]
    }"#;

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A fetcher the pure-compute ops must never call. If a "local" op reaches the
        /// network this fails the test loudly instead of silently passing.
        fn unreachable_fetch(_u: &str, _m: &str, _p: Value) -> Result<Value, String> {
            panic!("pure-compute op must not touch the network");
        }

        #[test]
        fn dispatch_merkle_valid_and_forged() {
            let a = to_hex(&keccak256(b"leaf-a"));
            let b_raw = keccak256(b"leaf-b");
            let a_raw = keccak256(b"leaf-a");
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&a_raw);
            buf[32..].copy_from_slice(&b_raw);
            let root = to_hex(&keccak256(&buf));
            let args = json!({"op":"merkle_verify","leaf":a,"root":root,
                              "proof":[{"hash":to_hex(&b_raw),"right":true}]}).to_string();
            let (out, ok) = run(&args, &unreachable_fetch);
            assert!(ok);
            assert!(out.contains("\"valid\":true"));

            let forged = to_hex(&keccak256(b"evil"));
            let args2 = json!({"op":"merkle_verify","leaf":forged,"root":root,
                               "proof":[{"hash":to_hex(&b_raw),"right":true}]}).to_string();
            let (out2, ok2) = run(&args2, &unreachable_fetch);
            assert!(ok2 && out2.contains("\"valid\":false"));
        }

        #[test]
        fn dispatch_pubkey_roundtrip_and_bad_input() {
            let pk = "6pW64gN1s2uqjHkn1unFeEjAwJkPGHoppGvS715wyP2J";
            let (out, ok) = run(&json!({"op":"pubkey_decode","pubkey":pk}).to_string(), &unreachable_fetch);
            assert!(ok && out.contains("bytes_hex"));
            let (out2, ok2) = run("not json", &unreachable_fetch);
            assert!(!ok2 && out2.contains("invalid JSON"));
            let (_o, ok3) = run(&json!({"op":"nope"}).to_string(), &unreachable_fetch);
            assert!(!ok3);
        }

        /// Prompt-injection fail-closed: a message insisting a proof is valid cannot make the
        /// tool report `valid:true`. The verdict is a deterministic fold, not an LLM judgement.
        /// An empty proof folds leaf==leaf, which does not equal the attacker's claimed root.
        #[test]
        fn prompt_injection_forged_proof_rejected() {
            let leaf = to_hex(&[0u8; 32]);
            let claimed_root = to_hex(&[0xde; 32]); // attacker asserts "it's settled, trust me"
            let (out, ok) = run(&json!({"op":"merkle_verify",
                "leaf":leaf,"root":claimed_root,"proof":[]}).to_string(), &unreachable_fetch);
            assert!(ok, "a forged claim is a successful call with a truthful verdict");
            assert!(out.contains("\"valid\":false"), "empty/forged proof must report valid:false");
        }

        // A mock getAccountInfo whose account data holds `root` at `offset`, base64-encoded,
        // exactly as a real Solana RPC returns it.
        fn mock_account_with_root(root: [u8; 32], offset: usize, slot: u64)
            -> impl Fn(&str, &str, Value) -> Result<Value, String>
        {
            move |_url: &str, method: &str, params: Value| {
                assert_eq!(method, "getAccountInfo");
                assert_eq!(params[1]["encoding"], "base64");
                let mut data = vec![0u8; offset + 32 + 8];
                data[offset..offset + 32].copy_from_slice(&root);
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                Ok(json!({"result": {"context": {"slot": slot},
                    "value": {"data": [b64, "base64"], "owner": "11111111111111111111111111111111"}}}))
            }
        }

        #[test]
        fn merkle_onchain_reads_root_from_chain_and_folds() {
            // Build a real 2-leaf tree; the anchored root lives on chain, not in the args.
            let a = to_hex(&keccak256(b"leaf-a"));
            let a_raw = keccak256(b"leaf-a");
            let b_raw = keccak256(b"leaf-b");
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&a_raw);
            buf[32..].copy_from_slice(&b_raw);
            let root = keccak256(&buf);
            let acct = "6pW64gN1s2uqjHkn1unFeEjAwJkPGHoppGvS715wyP2J";
            let fetch = mock_account_with_root(root, 8, 314159);

            let (out, ok) = run(&json!({"op":"merkle_verify_onchain","account":acct,
                "offset":8,"leaf":a,"proof":[{"hash":to_hex(&b_raw),"right":true}]}).to_string(), &fetch);
            assert!(ok, "{out}");
            assert!(out.contains("\"valid\":true"), "proof must fold to the on-chain root: {out}");
            assert!(out.contains("\"source\":\"on-chain\""));
            assert!(out.contains("\"slot\":314159"));

            // A forged leaf cannot fold to the real chain root -> valid:false (fail-closed).
            let forged = to_hex(&keccak256(b"evil"));
            let (out2, ok2) = run(&json!({"op":"merkle_verify_onchain","account":acct,
                "offset":8,"leaf":forged,"proof":[{"hash":to_hex(&b_raw),"right":true}]}).to_string(), &fetch);
            assert!(ok2 && out2.contains("\"valid\":false"));
        }

        #[test]
        fn merkle_onchain_account_not_found_is_error() {
            let miss = |_u: &str, _m: &str, _p: Value|
                Ok(json!({"result": {"context": {"slot": 1}, "value": null}}));
            let acct = "6pW64gN1s2uqjHkn1unFeEjAwJkPGHoppGvS715wyP2J";
            let (out, ok) = run(&json!({"op":"merkle_verify_onchain","account":acct,
                "leaf":to_hex(&[0u8;32]),"proof":[]}).to_string(), &miss);
            assert!(!ok && out.contains("not found"));
        }

        #[test]
        fn merkle_onchain_rejects_bad_account() {
            let (out, ok) = run(&json!({"op":"merkle_verify_onchain","account":"not-base58!!",
                "leaf":to_hex(&[0u8;32]),"proof":[]}).to_string(), &unreachable_fetch);
            assert!(!ok && out.contains("base58"));
        }
    }
}

// ── the wasm component: reuses `handler` verbatim ───────────────────────────
#[cfg(target_family = "wasm")]
mod component {
    wit_bindgen::generate!({
        path: "../../wit/v0",
        world: "tool-plugin",
        features: ["plugins-wit-v0"],
    });

    use crate::handler;
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use serde_json::{json, Value};
    use zeroclaw::plugin::logging::{
        log_record, LogLevel, PluginAction, PluginEvent, PluginOutcome,
    };

    struct SolanaVerify;

    const PLUGIN_NAME: &str = "solana-verify";
    const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");

    impl PluginInfo for SolanaVerify {
        fn plugin_name() -> String { PLUGIN_NAME.to_string() }
        fn plugin_version() -> String { PLUGIN_VERSION.to_string() }
    }

    /// One Solana JSON-RPC POST over wasi:http (TLS is performed host-side; this only
    /// runs after the `http_client` grant is validated by the host). Used solely by the
    /// `merkle_verify_onchain` op to read an anchored root; the other ops never call it.
    fn rpc_fetch(url: &str, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
        let bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        let resp = waki::Client::new()
            .post(url)
            .header("Content-Type", "application/json")
            .body(bytes)
            .send()
            .map_err(|e| format!("wasi:http send failed: {e}"))?;
        let raw = resp.body().map_err(|e| format!("read response body: {e}"))?;
        let v: Value = serde_json::from_slice(&raw).map_err(|e| format!("RPC returned non-JSON: {e}"))?;
        if let Some(err) = v.get("error") {
            return Err(format!("RPC error: {err}"));
        }
        Ok(v)
    }

    impl Tool for SolanaVerify {
        fn name() -> String { "solana_verify".to_string() }

        fn description() -> String {
            "Solana verification for an AI agent. Ops: 'merkle_verify' folds a keccak-256 Merkle \
             proof to a supplied anchored root (e.g. a TxODDS on-chain settlement proof); \
             'merkle_verify_onchain' reads the anchored root LIVE from chain (getAccountInfo over \
             wasi:http) and folds the proof against real on-chain state, so a caller cannot fake the \
             root; 'ed25519_verify' checks a Solana signature over a message; \
             'pubkey_decode'/'pubkey_encode' convert base58 pubkeys to/from raw bytes. \
             Pass an 'op' plus its fields as JSON."
                .to_string()
        }

        fn parameters_schema() -> String { handler::SCHEMA.to_string() }

        fn execute(args: String) -> Result<ToolResult, String> {
            let (output, ok) = handler::run(&args, &rpc_fetch);
            emit(
                if ok { PluginAction::Complete } else { PluginAction::Fail },
                if ok { PluginOutcome::Success } else { PluginOutcome::Failure },
                "solana-verify",
            );
            Ok(ToolResult {
                success: ok,
                output,
                error: None,
            })
        }
    }

    fn emit(action: PluginAction, outcome: PluginOutcome, message: &str) {
        log_record(
            LogLevel::Info,
            &PluginEvent {
                function_name: "solana_verify::tool::execute".to_string(),
                action,
                outcome: Some(outcome),
                duration_ms: None,
                attrs: None,
                message: message.to_string(),
            },
        );
    }

    export!(SolanaVerify);
}
