"""Phase 4 independent privacy verification of live testnet payout transactions.

Sources compared for every transaction:
  1. ZecBlock (testnet.zecblock.com, formerly CipherScan): a separately operated
     Zebra-compatible node + indexer + decoder. Fetched over HTTPS: decoded JSON
     and the raw transaction hex.
  2. testnet.zec.rocks lightwalletd, decoded by our own keyless observer
     (wallet-proof `observe`) -- the source used in Phase 3.
  3. The payout wallet's intent store: the exact signed bytes persisted BEFORE
     broadcast (treasury payouts only).

Checks: same txid, height, size, fee and shielded action counts from both
sources; no transparent inputs/outputs; raw bytes from ZecBlock identical to
the bytes the wallet persisted (sha256); header carries tx v6 + NU7 branch id;
and plaintext amount / memo / payout id / recipient UA do not appear in the
public bytes. Writes JSON per tx and prints a table. No keys are used.

Usage: python independent_verify.py <evidence_dir>
"""
import hashlib, json, os, sqlite3, struct, subprocess, sys, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CLI = os.path.join(ROOT, "wallet-proof", "target", "debug", "hushush-wallet-proof.exe")
INTENTS = os.path.join(os.environ["LOCALAPPDATA"], "hushush-phase0", "wallets", "treasury", "payout-intents.sqlite")
API = "https://api.testnet.zecblock.com/api"
NU7_BRANCH = 0x77190AD9

# txid -> (label, amount zats or None, payout id or None)
TXS = {
    "e466d8713b0d9b6af40fb38dd2a547c5fc4eea1b3adc98fdfe000dc74b0898bb": ("Phase 0 alice->bob", 150000, None),
    "98cb28bdfc8bc49f7ddc0224813dbfe7faa57b167d7a3ffe40a146071dbc4cd3": ("Phase 3 funding alice->treasury", 4000000, None),
    "31ca0061720a10cbc63823568530ed0ea39c13534a6f980f04951a5a3eb52a50": ("Phase 3 run1 payout A", 150000, None),
    "ef344ccd56f1f023e2cd0435a4ccd384d655d717d6eba01812405d8c6a4721b4": ("Phase 3 run1 payout B", 150000, None),
    "52682f9a124823bc6b07ed9e84e4fef5efafc50ce6bed188be34aa539d3440d9": ("Phase 3 run2 payout A", 150000, "6d531848-1c16-4517-bfef-5e5718bf4ef5"),
    "e804851e726086b4c0cdc43dd17a8664a0f754a8cb7393e7876b7362069065f6": ("Phase 3 run2 payout B (interrupted)", 150000, "f4e3ad8f-dde8-4cf5-852d-b323d991e6a0"),
    "e5f4d11788d3f848573b855142407dda213f610b72a53991a2bca524e4c6e71d": ("Phase 3 run2 payout C (crash before broadcast)", 150000, "ec815059-629e-4d1e-8e34-fa7ebae1948e"),
}
TXS.update(json.loads(os.environ.get("EXTRA_TXS", "{}")))


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": "hushush-phase4-verify"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def observer(txid):
    out = subprocess.run([CLI, "observe", txid], capture_output=True, text=True, timeout=120)
    return json.loads(out.stdout) if out.returncode == 0 else {"error": out.stderr.strip()[-200:]}


def intent_raw(txid):
    # The current intent store and any moved aside by the intent-loss test (G).
    d = os.path.dirname(INTENTS)
    for name in sorted(os.listdir(d)) if os.path.isdir(d) else []:
        if not name.startswith("payout-intents.sqlite") or name.endswith(("-wal", "-shm")):
            continue
        c = sqlite3.connect(f"file:{os.path.join(d, name)}?mode=ro", uri=True)
        r = c.execute("SELECT raw_tx FROM payment_intents WHERE txid = ?", (txid,)).fetchone()
        c.close()
        if r and r[0]:
            return bytes(r[0])
    return None


def main(evid):
    carol = subprocess.run([CLI, "address", "carol"], capture_output=True, text=True).stdout.strip()
    rows, ok_all = [], True
    for txid, (label, amount, payout_id) in TXS.items():
        ex = get(f"{API}/tx/{txid}")
        raw = bytes.fromhex(get(f"{API}/tx/{txid}/raw")["hex"])
        ob = observer(txid)
        header, vgid, branch = struct.unpack_from("<III", raw, 0)
        persisted = intent_raw(txid)
        needles = {"amount (8-byte LE)": struct.pack("<Q", amount) if amount else None,
                   "memo text 'hushush payout'": b"hushush payout",
                   "payout id": payout_id.encode() if payout_id else None,
                   "recipient UA text": carol.encode() if carol else None}
        leaks = [k for k, v in needles.items() if v and v in raw]
        checks = {
            "explorer: confirmed + canonical": ex.get("status") == "confirmed" and ex.get("isCanonical") is True,
            "explorer txid matches": ex.get("txid") == txid,
            "header: v6 + NU7 branch": (header & 0x7FFFFFFF) == 6 and branch == NU7_BRANCH,
            "explorer: no transparent in/out": ex.get("inputCount") == 0 and ex.get("outputCount") == 0 and ex.get("totalOutputZat") == "0",
            "height agrees (explorer vs zec.rocks)": str(ob.get("mined_height")) == str(ex.get("blockHeight")),
            "size agrees": ob.get("size_bytes") == ex.get("size") == len(raw),
            "fee agrees": str(ob.get("fee_zats")) == str(ex.get("feeZat")),
            "actions agree": (ob.get("ironwood_actions"), ob.get("orchard_actions"), ob.get("sapling_spends"), ob.get("sapling_outputs"))
                == (ex.get("ironwoodActions"), ex.get("orchardActions"), ex.get("saplingSpendCount"), ex.get("saplingOutputCount")),
            "no plaintext amount/memo/payout id/UA in bytes": not leaks,
        }
        if persisted is not None:
            checks["bytes == wallet's persisted pre-broadcast bytes"] = hashlib.sha256(persisted).digest() == hashlib.sha256(raw).digest()
        ok = all(checks.values())
        ok_all &= ok
        rec = {"txid": txid, "label": label, "explorer": ex, "explorer_raw_sha256": hashlib.sha256(raw).hexdigest(),
               "zec_rocks_observer": ob, "persisted_raw_sha256": hashlib.sha256(persisted).hexdigest() if persisted else None,
               "checks": checks, "plaintext_found": leaks}
        with open(os.path.join(evid, f"independent-{txid[:16]}.json"), "w") as f:
            json.dump(rec, f, indent=2)
        rows.append((label, txid, ex.get("blockHeight"), ex.get("confirmations"), ex.get("feeZat"),
                     f"ironwood={ex.get('ironwoodActions')} orchard={ex.get('orchardActions')} sapling={ex.get('saplingSpendCount')}/{ex.get('saplingOutputCount')}",
                     "same bytes" if persisted is not None and checks.get("bytes == wallet's persisted pre-broadcast bytes") else ("n/a" if persisted is None else "DIFFERENT"),
                     "PASS" if ok else "FAIL: " + ", ".join(k for k, v in checks.items() if not v)))
    for r in rows:
        print(" | ".join(str(x) for x in r))
    print("ALL PASS" if ok_all else "SOME CHECKS FAILED")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
