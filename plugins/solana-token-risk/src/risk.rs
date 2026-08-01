//! Pure, deterministic Solana token risk scoring — no wasm, no network here.
//!
//! Input is the *parsed* on-chain state of a mint (as returned by a Solana RPC
//! `getAccountInfo`/`getTokenLargestAccounts`/`getTokenSupply` with
//! `encoding: "jsonParsed"`). Output is a structured EVIDENCE report: every flag
//! names the raw on-chain fact that triggered it and what that fact *enables*, so
//! an agent (or a human) can act on it. It is deterministic — the same chain
//! state always yields the same verdict — so a prompt cannot argue a token safe.
//!
//! This is evidence, not financial advice. Holder concentration in particular can
//! reflect a liquidity pool or an exchange, not a malicious whale; the report says so.

use serde_json::Value;

/// The SPL burn incinerator — tokens here are provably out of circulation, so we
/// exclude it from "whale concentration".
const INCINERATOR: &str = "1nc1nerator11111111111111111111111111111111";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Flag {
    pub code: String,
    pub severity: Severity,
    /// What is wrong, in one line.
    pub title: String,
    /// The raw on-chain fact that triggered it.
    pub evidence: String,
    /// Points contributed to the 0-100 risk score.
    pub points: u32,
}

/// Normalized on-chain facts about a mint.
#[derive(Debug, Clone, Default)]
pub struct TokenFacts {
    pub mint: String,
    pub program: String, // "spl-token" | "spl-token-2022"
    pub is_initialized: bool,
    pub decimals: u8,
    pub raw_supply: u128,
    pub ui_supply: f64,
    pub mint_authority: Option<String>,
    pub freeze_authority: Option<String>,
    /// Token-2022 extensions, by their RPC `extension` name.
    pub extensions: Vec<Extension>,
    /// Largest token accounts (address, ui_amount), already sorted desc by amount.
    pub top_holders: Vec<(String, f64)>,
    pub holders_source_ok: bool,
}

#[derive(Debug, Clone)]
pub struct Extension {
    pub name: String,
    pub state: Value,
}

impl TokenFacts {
    fn ext(&self, name: &str) -> Option<&Extension> {
        self.extensions.iter().find(|e| e.name == name)
    }
}

// ── parsing RPC jsonParsed responses ───────────────────────────────────────

/// Accept either a full JSON-RPC envelope (`{"result": {...}}`) or the inner
/// value directly, and return the `result` (or the value itself).
fn unwrap_result(v: &Value) -> &Value {
    v.get("result").unwrap_or(v)
}

/// Parse a `getAccountInfo` (jsonParsed) response for a mint into TokenFacts.
pub fn parse_mint(mint: &str, account_info: &Value) -> Result<TokenFacts, String> {
    let result = unwrap_result(account_info);
    let value = result.get("value").unwrap_or(result);
    if value.is_null() {
        return Err("mint account not found on this RPC (null value) — wrong address or network?".into());
    }
    let data = value.get("data").ok_or("account has no `data` — is this a token mint?")?;
    let parsed = data
        .get("parsed")
        .ok_or("account data is not jsonParsed — call getAccountInfo with encoding:jsonParsed")?;
    let typ = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ != "mint" {
        return Err(format!("account is a `{typ}`, not a token `mint`"));
    }
    let program = data
        .get("program")
        .and_then(|p| p.as_str())
        .unwrap_or("spl-token")
        .to_string();
    let info = parsed.get("info").ok_or("mint has no `info`")?;

    let decimals = info.get("decimals").and_then(|d| d.as_u64()).unwrap_or(0) as u8;
    let raw_supply: u128 = info
        .get("supply")
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let ui_supply = raw_supply as f64 / 10f64.powi(decimals as i32);
    let is_initialized = info.get("isInitialized").and_then(|b| b.as_bool()).unwrap_or(true);

    // mintAuthority / freezeAuthority: a JSON `null` means renounced (good).
    let auth = |k: &str| -> Option<String> {
        match info.get(k) {
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        }
    };

    let mut extensions = Vec::new();
    if let Some(exts) = info.get("extensions").and_then(|e| e.as_array()) {
        for e in exts {
            if let Some(name) = e.get("extension").and_then(|n| n.as_str()) {
                extensions.push(Extension {
                    name: name.to_string(),
                    state: e.get("state").cloned().unwrap_or(Value::Null),
                });
            }
        }
    }

    Ok(TokenFacts {
        mint: mint.to_string(),
        program,
        is_initialized,
        decimals,
        raw_supply,
        ui_supply,
        mint_authority: auth("mintAuthority"),
        freeze_authority: auth("freezeAuthority"),
        extensions,
        top_holders: Vec::new(),
        holders_source_ok: false,
    })
}

/// Fold a `getTokenLargestAccounts` response into the facts (top holders).
pub fn apply_largest(facts: &mut TokenFacts, largest: &Value) {
    let result = unwrap_result(largest);
    let arr = match result.get("value").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return,
    };
    let mut holders: Vec<(String, f64)> = Vec::new();
    for a in arr {
        let addr = a.get("address").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let ui = a
            .get("uiAmount")
            .and_then(|x| x.as_f64())
            .or_else(|| a.get("uiAmountString").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()))
            .unwrap_or(0.0);
        if !addr.is_empty() {
            holders.push((addr, ui));
        }
    }
    holders.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    facts.top_holders = holders;
    facts.holders_source_ok = true;
}

// ── scoring ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RiskReport {
    pub flags: Vec<Flag>,
    pub score: u32, // 0-100
    pub band: &'static str,
    pub notes: Vec<String>,
}

fn band_for(score: u32, max_sev: Severity) -> &'static str {
    // Score sets the floor; a single critical/high fact raises it so a lone but
    // fatal flag can't be averaged away.
    let by_score = match score {
        s if s >= 60 => "CRITICAL",
        s if s >= 35 => "HIGH",
        s if s >= 15 => "MEDIUM",
        s if s >= 1 => "LOW",
        _ => "MINIMAL",
    };
    let by_sev = match max_sev {
        Severity::Critical => "CRITICAL",
        Severity::High => "HIGH",
        Severity::Medium => "MEDIUM",
        Severity::Low => "LOW",
        Severity::Info => "MINIMAL",
    };
    // Return the more severe of the two.
    let rank = |b: &str| match b {
        "CRITICAL" => 4,
        "HIGH" => 3,
        "MEDIUM" => 2,
        "LOW" => 1,
        _ => 0,
    };
    if rank(by_sev) >= rank(by_score) { by_sev } else { by_score }
}

/// Assess a mint from its normalized facts. Deterministic.
pub fn assess(f: &TokenFacts) -> RiskReport {
    let mut flags: Vec<Flag> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    if !f.is_initialized {
        flags.push(Flag {
            code: "mint_uninitialized".into(),
            severity: Severity::High,
            title: "Mint is not initialized".into(),
            evidence: "isInitialized = false".into(),
            points: 20,
        });
    }

    // Authorities.
    if let Some(a) = &f.mint_authority {
        flags.push(Flag {
            code: "mint_authority_present".into(),
            severity: Severity::Critical,
            title: "Mint authority is live — supply can be inflated at will".into(),
            evidence: format!("mintAuthority = {a} (not null). New tokens can be minted, diluting holders."),
            points: 35,
        });
    } else {
        notes.push("Mint authority is renounced (null) — supply is fixed.".into());
    }
    if let Some(a) = &f.freeze_authority {
        flags.push(Flag {
            code: "freeze_authority_present".into(),
            severity: Severity::High,
            title: "Freeze authority is live — accounts can be frozen (sell can be blocked)".into(),
            evidence: format!("freezeAuthority = {a} (not null). Holder token accounts can be frozen, a classic honeypot."),
            points: 25,
        });
    } else {
        notes.push("Freeze authority is renounced (null) — accounts cannot be frozen.".into());
    }

    // Token-2022 dangerous extensions.
    if f.program == "spl-token-2022" {
        if f.ext("transferHook").is_some() {
            flags.push(Flag {
                code: "transfer_hook".into(),
                severity: Severity::Critical,
                title: "Transfer hook — arbitrary program runs on every transfer".into(),
                evidence: "Token-2022 `transferHook` extension is set. The hook can revert transfers (block sells) or add logic on each move.".into(),
                points: 40,
            });
        }
        if f.ext("permanentDelegate").is_some() {
            flags.push(Flag {
                code: "permanent_delegate".into(),
                severity: Severity::Critical,
                title: "Permanent delegate — an authority can move or burn anyone's tokens".into(),
                evidence: "Token-2022 `permanentDelegate` extension is set. The delegate can transfer/burn tokens from any holder without consent.".into(),
                points: 40,
            });
        }
        if f.ext("nonTransferable").is_some() {
            flags.push(Flag {
                code: "non_transferable".into(),
                severity: Severity::Critical,
                title: "Non-transferable — tokens cannot be sold or moved at all".into(),
                evidence: "Token-2022 `nonTransferable` extension is set. Holders can never transfer; a total honeypot.".into(),
                points: 45,
            });
        }
        if let Some(e) = f.ext("transferFeeConfig") {
            let (bps, authority_can_raise) = transfer_fee(&e.state);
            let pct = bps as f64 / 100.0;
            let (sev, pts) = match bps {
                0..=100 => (Severity::Low, 8),
                101..=500 => (Severity::Medium, 18),
                _ => (Severity::High, 30),
            };
            flags.push(Flag {
                code: "transfer_fee".into(),
                severity: sev,
                title: format!("Transfer fee of {pct:.2}% on every trade"),
                evidence: format!(
                    "Token-2022 `transferFeeConfig`: {bps} bps current fee{}.",
                    if authority_can_raise { ", with a live fee authority that can raise it (up to 100%)" } else { "" }
                ),
                points: if authority_can_raise { pts + 10 } else { pts },
            });
        }
        if let Some(e) = f.ext("defaultAccountState") {
            let frozen = e.state.get("accountState").and_then(|s| s.as_str()) == Some("frozen");
            if frozen {
                flags.push(Flag {
                    code: "default_account_state_frozen".into(),
                    severity: Severity::High,
                    title: "New accounts default to FROZEN — you can't move tokens until an authority thaws you".into(),
                    evidence: "Token-2022 `defaultAccountState` = frozen. Every new holder is frozen by default; the authority decides who may transfer.".into(),
                    points: 28,
                });
            }
        }
        if f.ext("mintCloseAuthority").is_some() {
            flags.push(Flag {
                code: "mint_close_authority".into(),
                severity: Severity::Medium,
                title: "Mint can be closed by an authority".into(),
                evidence: "Token-2022 `mintCloseAuthority` is set — the mint account can be closed.".into(),
                points: 12,
            });
        }
    }

    // Holder concentration (best-effort; excludes the burn incinerator).
    if f.holders_source_ok && f.ui_supply > 0.0 && !f.top_holders.is_empty() {
        let mut top_addr = String::new();
        let mut top_amt = 0.0f64;
        let mut top5 = 0.0f64;
        let mut counted = 0;
        for (addr, amt) in &f.top_holders {
            if addr == INCINERATOR {
                continue; // burned supply is not a whale
            }
            if counted == 0 {
                top_addr = addr.clone();
                top_amt = *amt;
            }
            if counted < 5 {
                top5 += *amt;
            }
            counted += 1;
        }
        let top_pct = 100.0 * top_amt / f.ui_supply;
        let top5_pct = 100.0 * top5 / f.ui_supply;
        let (sev, pts) = match top_pct {
            p if p >= 90.0 => (Severity::High, 30),
            p if p >= 50.0 => (Severity::Medium, 20),
            p if p >= 30.0 => (Severity::Low, 10),
            _ => (Severity::Info, 0),
        };
        if pts > 0 {
            flags.push(Flag {
                code: "holder_concentration".into(),
                severity: sev,
                title: format!("Top holder controls {top_pct:.1}% of supply"),
                evidence: format!(
                    "Largest token account {top_addr} holds {top_pct:.1}%; top-5 hold {top5_pct:.1}%. A single sell can crater the price."
                ),
                points: pts,
            });
        }
        notes.push(
            "Holder concentration is best-effort: the largest account may be a liquidity pool or exchange, not a malicious whale. Verify the owner before concluding.".into(),
        );
    } else {
        notes.push("Holder concentration not evaluated (largest-accounts data unavailable).".into());
    }

    let score = flags.iter().map(|f| f.points).sum::<u32>().min(100);
    let max_sev = flags.iter().map(|f| f.severity).max().unwrap_or(Severity::Info);
    let band = band_for(score, max_sev);

    RiskReport { flags, score, band, notes }
}

/// Read the current transfer-fee bps and whether a fee authority can still raise it.
fn transfer_fee(state: &Value) -> (u64, bool) {
    let bps = state
        .get("newerTransferFee")
        .and_then(|n| n.get("transferFeeBasisPoints"))
        .and_then(|b| b.as_u64())
        .or_else(|| state.get("transferFeeBasisPoints").and_then(|b| b.as_u64()))
        .unwrap_or(0);
    let authority_can_raise = matches!(
        state.get("transferFeeConfigAuthority"),
        Some(Value::String(s)) if !s.is_empty()
    );
    (bps, authority_can_raise)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mint_resp(program: &str, mint_auth: Value, freeze_auth: Value, exts: Value) -> Value {
        json!({"result":{"value":{"data":{"parsed":{"info":{
            "decimals":6,"isInitialized":true,"supply":"1000000000000",
            "mintAuthority":mint_auth,"freezeAuthority":freeze_auth,"extensions":exts
        },"type":"mint"},"program":program},"owner":"x"}}})
    }

    #[test]
    fn clean_renounced_token_is_minimal_risk() {
        let resp = mint_resp("spl-token", Value::Null, Value::Null, Value::Null);
        let f = parse_mint("Mint111", &resp).unwrap();
        let r = assess(&f);
        assert_eq!(r.band, "MINIMAL");
        assert_eq!(r.score, 0);
        assert!(r.flags.is_empty());
        assert!(r.notes.iter().any(|n| n.contains("Mint authority is renounced")));
    }

    #[test]
    fn live_mint_authority_is_critical() {
        let resp = mint_resp("spl-token", json!("Boss1111"), Value::Null, Value::Null);
        let f = parse_mint("Mint111", &resp).unwrap();
        let r = assess(&f);
        assert_eq!(r.band, "CRITICAL");
        assert!(r.flags.iter().any(|fl| fl.code == "mint_authority_present" && fl.severity == Severity::Critical));
    }

    #[test]
    fn freeze_authority_flagged_high() {
        let resp = mint_resp("spl-token", Value::Null, json!("Freezer1"), Value::Null);
        let f = parse_mint("Mint111", &resp).unwrap();
        let r = assess(&f);
        assert!(r.flags.iter().any(|fl| fl.code == "freeze_authority_present"));
        assert_eq!(r.band, "HIGH");
    }

    #[test]
    fn token2022_transfer_hook_and_permanent_delegate_critical() {
        let exts = json!([
            {"extension":"transferHook","state":{"authority":"a","programId":"p"}},
            {"extension":"permanentDelegate","state":{"delegate":"d"}}
        ]);
        let resp = mint_resp("spl-token-2022", Value::Null, Value::Null, exts);
        let f = parse_mint("Mint111", &resp).unwrap();
        let r = assess(&f);
        assert!(r.flags.iter().any(|fl| fl.code == "transfer_hook"));
        assert!(r.flags.iter().any(|fl| fl.code == "permanent_delegate"));
        assert_eq!(r.band, "CRITICAL");
    }

    #[test]
    fn transfer_fee_reads_bps_and_authority() {
        let exts = json!([{"extension":"transferFeeConfig","state":{
            "newerTransferFee":{"transferFeeBasisPoints":800},
            "transferFeeConfigAuthority":"FeeBoss"
        }}]);
        let resp = mint_resp("spl-token-2022", Value::Null, Value::Null, exts);
        let f = parse_mint("Mint111", &resp).unwrap();
        let r = assess(&f);
        let fee = r.flags.iter().find(|fl| fl.code == "transfer_fee").unwrap();
        assert_eq!(fee.severity, Severity::High); // 800 bps
        assert!(fee.evidence.contains("800 bps"));
        assert!(fee.evidence.contains("can raise it"));
    }

    #[test]
    fn holder_concentration_excludes_burn_and_flags_whale() {
        let mut resp = mint_resp("spl-token", Value::Null, Value::Null, Value::Null);
        // supply is 1,000,000 ui (1e12 raw / 1e6)
        let _ = &mut resp;
        let largest = json!({"result":{"value":[
            {"address": INCINERATOR, "uiAmount": 400000.0},
            {"address":"Whale1","uiAmount":550000.0},
            {"address":"Small1","uiAmount":50000.0}
        ]}});
        let mut f = parse_mint("Mint111", &resp).unwrap();
        apply_largest(&mut f, &largest);
        let r = assess(&f);
        let conc = r.flags.iter().find(|fl| fl.code == "holder_concentration").unwrap();
        // Whale holds 550k of 1,000k = 55% (burn excluded), MEDIUM band.
        assert!(conc.evidence.contains("55.0%"));
        assert_eq!(conc.severity, Severity::Medium);
    }

    #[test]
    fn parse_rejects_non_mint() {
        let resp = json!({"result":{"value":{"data":{"parsed":{"type":"account","info":{}},"program":"spl-token"}}}});
        assert!(parse_mint("x", &resp).is_err());
    }

    #[test]
    fn parse_rejects_missing_account() {
        let resp = json!({"result":{"value":null}});
        assert!(parse_mint("x", &resp).is_err());
    }
}
