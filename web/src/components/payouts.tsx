import type { PayoutView } from "@/lib/payroll";
import { compactZec, formatTime } from "@/lib/format";
import { Badge } from "./ui";

/** Average Zcash block time, for the "≈ minutes left" hint only. */
const BLOCK_SECS = 75;

export function Txid({ txid }: { txid: string }) {
  return (
    <span className="font-mono text-xs" title={txid}>
      {txid.slice(0, 10)}…{txid.slice(-6)}
    </span>
  );
}

/**
 * A withdrawal's state in plain words. Only `confirmed` is "Paid": a
 * broadcast transaction is not a payment until it has enough confirmations.
 */
export function PayoutBadge({ p }: { p: PayoutView }) {
  switch (p.state) {
    case "reserved":
      return <Badge tone="amber" title="Building the shielded transaction. No ZEC has been sent yet.">Sending…</Badge>;
    case "submitted": {
      const n = Math.min(p.confirmations, p.required_confirmations);
      return (
        <Badge tone="accent" title="On the network; it counts as paid after enough confirmations.">
          Confirming {n}/{p.required_confirmations}
        </Badge>
      );
    }
    case "confirmed":
      return <Badge tone="green">Paid ✓</Badge>;
    case "failed":
      return <Badge tone="red" title="No ZEC was sent. The amount is available again.">Not sent</Badge>;
    case "requires_reconciliation":
      return <Badge tone="amber" title="The outcome is being verified automatically. It will never be paid twice.">Checking…</Badge>;
  }
}

export function PayoutDetail({ p }: { p: PayoutView }) {
  const fee = p.fee_zec ? ` · network fee ${compactZec(p.fee_zec)} ZEC (paid by the employer)` : "";
  switch (p.state) {
    case "reserved":
      return <>Building the shielded transaction — this takes about a minute. No ZEC has been sent yet.</>;
    case "submitted": {
      const left = Math.max(0, p.required_confirmations - p.confirmations);
      return (
        <>
          {p.txid && <Txid txid={p.txid} />} is on the testnet{p.mined_height ? ` (block ${p.mined_height})` : ", waiting to be mined"}.
          {left > 0 && <> About {Math.max(1, Math.round((left * BLOCK_SECS) / 60))} min until it counts as paid.</>}
          {fee}
        </>
      );
    }
    case "confirmed":
      return (
        <>
          Paid {formatTime(p.resolved_at)} in {p.txid && <Txid txid={p.txid} />} (block {p.mined_height}). Amount and recipient verified{fee}.
        </>
      );
    case "failed":
      return <>Not sent: {p.detail ?? "the payout failed"}. The amount is available to withdraw again.</>;
    case "requires_reconciliation":
      return (
        <>
          {p.txid && <><Txid txid={p.txid} />: </>}the outcome is being verified ({p.detail ?? "ambiguous result"}). It will never be paid twice.
        </>
      );
  }
}

export function PayoutList({ payouts, empty = "No withdrawals yet." }: { payouts: PayoutView[]; empty?: string }) {
  if (payouts.length === 0) return <p className="text-sm text-ink-soft">{empty}</p>;
  return (
    <ul className="divide-y divide-line">
      {payouts.map((p) => (
        <li key={p.id} className="flex flex-col gap-1 py-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
          <div className="min-w-0">
            <div className="font-mono text-sm font-semibold tabular-nums">{compactZec(p.amount_zec)} ZEC</div>
            <div className="mt-0.5 max-w-prose text-xs text-ink-soft">
              <PayoutDetail p={p} />
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2 text-xs text-ink-soft">
            {formatTime(p.created_at)}
            <PayoutBadge p={p} />
          </div>
        </li>
      ))}
    </ul>
  );
}
