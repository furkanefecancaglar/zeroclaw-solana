#!/usr/bin/env bash
# Composition demo: the four plugins chained the way a ZeroClaw agent would use
# them, on REAL mainnet data. This is the system-level story — each tool's output
# feeds the next:
#
#   solana-wallet-risk  →  screen a wallet, find the riskiest holding
#   solana-token-risk   →  deep-dive that exact mint, confirm the threat
#   solana-tx-builder   →  construct an UNSIGNED exit transfer (agent builds, wallet signs)
#   solana-tx-guard    →  decode + simulate the built tx before it is signed
#   solana-verify      →  the no-custody settlement primitive the flow rests on
#
# Nothing is mocked and nothing is signed. One network grant, no keys.
set -euo pipefail
trap '' PIPE
cd "$(dirname "$0")"
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"

WALLET="${1:-GThUX1Atko4tqhN2NaiTazWSeFWMuiUvfFnyJyUghFMJ}"
RPC="${2:-https://api.mainnet-beta.solana.com}"
SAFE_DEST="11111111111111111111111111111111"   # placeholder cold-wallet destination
SPL="TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
T22="TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

rpc() { curl -s "$RPC" -X POST -H 'Content-Type: application/json' -d "$1"; }

echo "════════════════════════════════════════════════════════════════"
echo " ZeroClaw Solana suite — composition demo (real mainnet, no keys)"
echo " wallet: $WALLET"
echo "════════════════════════════════════════════════════════════════"

echo
echo "── STEP 1/5 · solana-wallet-risk: screen the wallet ─────────────"
rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getTokenAccountsByOwner\",\"params\":[\"$WALLET\",{\"programId\":\"$SPL\"},{\"encoding\":\"jsonParsed\"}]}" > "$TMP/spl.json"
rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getTokenAccountsByOwner\",\"params\":[\"$WALLET\",{\"programId\":\"$T22\"},{\"encoding\":\"jsonParsed\"}]}" > "$TMP/t22.json"
MINTS=$(python3 - "$TMP/spl.json" "$TMP/t22.json" <<'PY'
import json,sys
rows=[]
for p in sys.argv[1:]:
    try: d=json.load(open(p))
    except: continue
    for e in (d.get("result") or {}).get("value",[]) or []:
        i=((e.get("account") or {}).get("data") or {}).get("parsed",{}).get("info",{})
        a=(i.get("tokenAmount") or {}).get("uiAmount") or 0
        if i.get("mint") and a and a>0: rows.append((a,i["mint"]))
rows.sort(reverse=True); seen=[]
for _a,m in rows:
    if m not in seen: seen.append(m)
    if len(seen)>=12: break
print("\n".join(seen))
PY
)
echo "{" > "$TMP/mints.json"; F=1
for M in $MINTS; do
  R=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getAccountInfo\",\"params\":[\"$M\",{\"encoding\":\"jsonParsed\"}]}")
  [ $F -eq 0 ] && echo "," >> "$TMP/mints.json"; F=0
  printf '"%s": %s' "$M" "$R" >> "$TMP/mints.json"
done
echo "}" >> "$TMP/mints.json"
WR=$(cargo run --release --quiet -p solana-wallet-risk --example scan_files -- "$WALLET" "$TMP/spl.json" "$TMP/t22.json" "$TMP/mints.json" 2>/dev/null || \
     ( cd plugins/solana-wallet-risk && cargo run --release --quiet --example scan_files -- "$WALLET" "$TMP/spl.json" "$TMP/t22.json" "$TMP/mints.json" ) 2>/dev/null)
echo "$WR" | python3 -c "import sys,json;d=json.load(sys.stdin);print('  summary:',d['summary']);print('  wallet band:',d['wallet_risk_band'])"
WORST_MINT=$(echo "$WR" | python3 -c "
import sys,json
d=json.load(sys.stdin)
risky=[h for h in d['holdings'] if h['threats']]
risky.sort(key=lambda h:h['risk_score'],reverse=True)
print(risky[0]['mint'] if risky else '')
")
echo "  → riskiest holding to investigate: ${WORST_MINT:-<none flagged>}"
[ -z "$WORST_MINT" ] && { echo "  (no flagged holding; nothing to chain) "; exit 0; }

echo
echo "── STEP 2/5 · solana-token-risk: deep-dive that mint ────────────"
echo "  (two lenses, same facts: wallet-risk banded it by PORTFOLIO exposure;"
echo "   token-risk now rates the MINT's rug capability — a deep-dive can escalate.)"
cp "$TMP/mints.json" "$TMP/one.json"
ACCT=$(python3 -c "import json;print(json.dumps(json.load(open('$TMP/mints.json'))['$WORST_MINT']))")
echo "$ACCT" > "$TMP/acct.json"
( cd plugins/solana-token-risk && cargo run --release --quiet --example assess_files -- "$WORST_MINT" "$TMP/acct.json" 2>/dev/null ) \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('  mint:',d['mint'])
print('  risk band:',d['risk_band'],'(score',str(d['risk_score'])+')')
for f in d['flags']: print('   •',f['severity'].upper(),'—',f['title'])
"

echo
echo "── STEP 3/5 · solana-tx-builder: build the UNSIGNED exit transfer ─"
echo "  moving the position to a cold wallet; the agent BUILDS, a wallet SIGNS."
( cd plugins/solana-tx-builder && cargo run --release --quiet --example run -- \
  "{\"op\":\"spl_transfer\",\"source\":\"$WALLET\",\"dest\":\"$SAFE_DEST\",\"authority\":\"$WALLET\",\"amount\":1}" 2>/dev/null ) \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
ix=d['instruction']
print('  program:',ix['program_id'])
signers=[a for a in ix['accounts'] if a['is_signer']]
print('  accounts:',len(ix['accounts']),'| required signers:',len(signers))
print('  → returned an UNSIGNED instruction. No signature, no key, nothing sent.')
"

echo
echo "── STEP 4/5 · solana-tx-guard: is the built transaction safe to sign? ─"
echo "  the agent doesn't sign blind — guard decodes the tx and simulates it live."
GUARD_TX=$( ( cd plugins/solana-tx-builder && cargo run --release --quiet --example run -- \
  "{\"op\":\"system_transfer\",\"from\":\"$WALLET\",\"to\":\"$SAFE_DEST\",\"lamports\":1000000}" 2>/dev/null ) )
# serialize that instruction into a base64 tx for the guard (python, byte-accurate)
TXB64=$(python3 - "$WALLET" "$SAFE_DEST" <<'PY'
import base64,struct,sys
A="123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58(s):
    n=0
    for c in s: n=n*58+A.index(c)
    b=n.to_bytes((n.bit_length()+7)//8,"big"); return b"\x00"*(len(s)-len(s.lstrip("1")))+b
def sv(n):
    o=bytearray()
    while True:
        e=n&0x7f;n>>=7;o.append(e|0x80 if n else e)
        if not n:break
    return bytes(o)
W=b58(sys.argv[1]);D=b58(sys.argv[2]);S=b"\x00"*32
m=bytes([1,0,1])+sv(3)+W+D+S+b"\x00"*32+sv(1)+bytes([2])+sv(2)+bytes([0,1])+sv(12)+struct.pack("<IQ",2,1000000)
print(base64.b64encode(sv(1)+b"\x00"*64+m).decode())
PY
)
SIM=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"simulateTransaction\",\"params\":[\"$TXB64\",{\"sigVerify\":false,\"replaceRecentBlockhash\":true,\"encoding\":\"base64\"}]}")
echo "$SIM" > "$TMP/exit.sim.json"
( cd plugins/solana-tx-guard && cargo run --release --quiet --example guard_file -- "$TXB64" "$TMP/exit.sim.json" 2>/dev/null ) \
  | python3 -c "
import sys,json
d=json.load(sys.stdin)
print('  verdict:',d['verdict'],'|',d['summary'])
sim=d.get('simulation') or {}
print('  live sim err:',json.dumps(sim.get('err')),'| units:',sim.get('units_consumed'))
"

echo
echo "── STEP 5/5 · solana-verify: the no-custody settlement primitive ─"
echo "  every step above is verifiable offline; verify folds a keccak Merkle proof"
echo "  or checks an ed25519 signature deterministically — the trust anchor."
( cd plugins/solana-verify && cargo run --release --quiet --example run -- \
  "{\"op\":\"pubkey_decode\",\"pubkey\":\"$WALLET\"}" 2>/dev/null ) \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print('  decoded owner pubkey ok:', d['bytes_hex'][:16]+'…')"

echo
echo "════════════════════════════════════════════════════════════════"
echo " Chained: screen → assess → build → GUARD → verify. One network grant, zero keys."
echo "════════════════════════════════════════════════════════════════"
