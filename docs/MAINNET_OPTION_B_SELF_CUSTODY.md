# Mainnet option B: self-custody payouts (employer signs each payout)

**Status:** design only, not built. Written 15 Oct 2026 alongside Phase 6
(per-company payout wallets on testnet, option A). This document records the
alternative for mainnet, where hushush never holds an employer's spending key.

## 1. The problem it solves

In option A (built for testnet in Phase 6), every company has its own account
in the hushush payout wallet. hushush derives and holds the spending keys, so it
can pay a worker the moment they tap **Withdraw**, even at night or on a
weekend. That is the product's core promise, and it works well on testnet.

On mainnet, holding the spending keys has two costs:

- **Security.** A server that can spend for every company is a single, very
  attractive target. A breach can drain every company at once.
- **Regulation.** Holding customers' money and paying it out on their behalf is
  custody. In most jurisdictions it needs a licence (money transmission or
  e-money, or a VASP/CASP registration such as under the EU's MiCA), plus
  compliance work: KYC, AML monitoring, audits and safeguarding of funds.

Option B removes both: **the employer keeps their own keys, and hushush
prepares payouts that the employer signs in their own wallet.**

## 2. What Zcash allows, and what it doesn't

- **No smart contracts, so no on-chain escrow or streaming.** The stream, its
  accrual and its milestones have to live off-chain (in hushush, as today).
  Only the final payout is a transaction.
- **No "connect wallet and let the site spend".** There is no Zcash equivalent
  of an approved ERC-20 allowance that a website can draw on later. Every
  spend needs the holder of the spending key to authorise it.
- **What exists: PCZT (Partially Created Zcash Transactions, ZIP 374 draft).** A
  service builds an unsigned transaction, a wallet adds proofs and signatures,
  and either side broadcasts. The librustzcash stack hushush already uses
  supports PCZT. On the wallet side, Zashi and the Keystone hardware wallet
  support signing PCZTs (Keystone through Zashi's QR flow).
- **Viewing keys.** An employer can share a Unified Full Viewing Key (UFVK)
  with hushush. With it, hushush can see the employer wallet's balance and
  build transactions from its notes, but it **cannot spend**.

## 3. How option B would work

### Setup (once per company)

1. The employer creates a payroll wallet in Zashi (or another PCZT-capable
   wallet), ideally a dedicated account, and funds it.
2. In hushush they export and paste its **UFVK**, or scan it as a QR code.
3. hushush imports the UFVK as a **view-only account** (`import_account_ufvk`
   in `zcash_client_sqlite`) and scans for it like any other account. The
   dashboard shows the real balance, and nothing can be spent from it.

### Each withdrawal

1. The worker taps **Withdraw**. As today, the engine reserves the amount
   (exactly-once, idempotency key, locked address) and creates the payout.
2. Instead of signing, the payout service **builds a PCZT** from the
   employer's view-only account: the inputs (notes), the output to the
   worker's locked address with the `hushush payout <id>` memo, change back to
   the employer, and the ZIP 317 fee.
3. The employer gets a notification (email, push or dashboard badge):
   "Ava requested 0.36 ZEC. Approve in your wallet." They open the request,
   which shows the PCZT as a QR code or deep link. They check it in Zashi or
   Keystone, which display the amount, recipient and fee themselves, and sign.
4. The signed PCZT comes back: hushush scans it from the wallet, or the wallet
   posts it to a hushush endpoint. hushush **verifies it is exactly the
   transaction it built** (same txid-relevant data, recipient, amount and
   memo), broadcasts it, and tracks confirmations with the existing payout
   state machine.
5. The worker sees "Waiting for Kestrel Collective to approve" instead of
   "Sending…". A payout is still **paid only once it is confirmed**.

### New payout states

The engine's payout states gain one before `submitted`:

```
reserved ──▶ awaiting_approval ──▶ submitted ──▶ confirmed
    │               │                    ╰──────▶ failed / requires_reconciliation
    │               ├─▶ expired   (employer didn't sign before the PCZT's expiry height)
    │               ╰─▶ rejected  (employer declined)
    ╰──▶ failed
```

`expired` and `rejected` release the reservation, so the amount becomes
available again, as a failed payout does today. A PCZT pins specific notes,
so if the employer spends from that wallet some other way, the PCZT becomes
invalid and is rebuilt. hushush should warn employers to use the wallet only
for payroll.

## 4. Trade-offs

| | Option A: hushush-held accounts (testnet now) | Option B: employer self-custody |
|---|---|---|
| Who holds spending keys | hushush (per-company accounts from one seed) | The employer, in their own wallet |
| Worker withdraws at night or on a weekend | Paid within minutes | Waits until the employer approves |
| Breach of hushush servers | Can drain every company's account | Can't spend anything (view keys only) |
| Licence / custody obligations | Likely required | Largely avoided: hushush never controls funds (get legal advice) |
| Employer effort | Fund once, top up when low | Approve every withdrawal |
| Privacy | hushush sees everything (it holds the keys) | hushush still sees amounts (UFVK), but can't move funds |
| Engineering | Done (Phase 6) | PCZT build/verify, approval UX, notifications, new states |

The key product consequence: **with option B, "withdraw any time" becomes
"request any time, paid when approved".** Streaming accrual and milestones
still work. Only the payout step depends on the employer.

## 5. Ways to soften the approval burden

- **Batching.** Approve all pending withdrawals in one transaction (one PCZT,
  several outputs) once a day. That's fewer approvals and lower total fees.
  The cost is that withdrawals become daily, not instant.
- **Approval windows and reminders.** Escalating notifications, and a
  dashboard counter of "waiting for approval".
- **Hybrid, the likely end state.** Let each employer choose:
  - **Managed:** option A with mainnet-grade custody (HSM/KMS-held keys, per-company
    spending limits, withdrawal velocity limits, insurance and the needed
    licences) for instant withdrawals.
  - **Self-custody:** option B for employers who want full control and accept
    approvals.
- **Hardware signing.** With Keystone, approvals become a QR scan on an
  air-gapped device. That's strong security for larger payrolls.

## 6. What would have to be built for option B

1. **Payout wallet:** view-only accounts from an imported UFVK, PCZT
   construction for a payout (`zcash_client_backend` PCZT support), and
   verification that a returned signed PCZT matches what was built before
   extracting and broadcasting it.
2. **Engine:** the `awaiting_approval`, `expired` and `rejected` states, an
   expiry-height check, and release of the reservation on expiry or
   rejection. Exactly-once guarantees stay as they are: one payout per
   reservation, one transaction per payout id.
3. **payroll-api:** endpoints to fetch a pending PCZT and submit the signed
   one, restricted to the company's managers.
4. **Web:**
   - onboarding to connect a payroll wallet (UFVK paste or QR)
   - an "Approvals" list with QR codes and deep links
   - the worker-side "waiting for approval" state
   - notifications
5. **Wallet compatibility testing** with Zashi and Keystone on testnet first,
   including what each wallet displays before signing, so employers can
   check the recipient and amount on their own device.
6. **Legal review** to confirm that the self-custody mode doesn't count as
   custody where hushush operates.

## 7. Recommendation

- **Testnet now:** option A (built in Phase 6). It keeps the "withdraw any
  time" experience and lets the product be tested end to end.
- **Before mainnet:** decide the custody model deliberately and get legal
  advice. If hushush launches with managed accounts, that needs proper key
  management (HSM/KMS), per-company spend limits, monitoring and licensing.
  Build option B as the **self-custody mode**: it's the right answer for
  employers who don't want a third party holding payroll funds, and it
  lowers hushush's regulatory exposure.

Open questions to settle before building option B:

- Which wallets to support at launch: Zashi only, or Zashi plus Keystone?
- Is daily batching acceptable as the default, or must every withdrawal be
  approved one by one?
- How long does an unsigned payout wait before it expires: hours, or a day?
- Does hushush need its own mobile app for push notifications, or is email
  enough at first?
