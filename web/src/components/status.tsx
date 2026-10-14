import type { ClaimView, StreamStatus } from "@/lib/payroll";
import { Badge } from "./ui";

export function StreamStatusBadge({ status }: { status: StreamStatus }) {
  if (status === "active") return <Badge tone="green">● Active</Badge>;
  if (status === "paused") return <Badge tone="amber">❚❚ Paused</Badge>;
  return <Badge>Ended</Badge>;
}

/**
 * Claim status labels. A reserved claim is ALWAYS shown as unpaid, including
 * while its payout transaction is in flight: a broadcast is not a payment.
 * "Paid" appears only for a settled claim, which only a payout confirmed on
 * chain (under the server's confirmation policy) can produce.
 */
export function ClaimStatusBadge({ claim }: { claim: Pick<ClaimView, "status" | "settlement_ref" | "payout"> }) {
  const p = claim.payout;
  if (claim.status === "reserved") {
    if (p?.state === "submitted") {
      return (
        <Badge tone="accent" title="The transaction is on the network. It counts as paid only after enough confirmations.">
          Sent · {Math.min(p.confirmations, p.required_confirmations)}/{p.required_confirmations} confirmations
        </Badge>
      );
    }
    if (p?.state === "reserved") return <Badge tone="amber" title="The payout is being prepared. No ZEC has been sent yet.">Payout starting · unpaid</Badge>;
    if (p?.state === "requires_reconciliation") {
      return (
        <Badge tone="amber" title="The payout's outcome is being verified. It will never be sent twice.">
          Checking payout · unpaid
        </Badge>
      );
    }
    return (
      <Badge tone="amber" title="Reserved against earned wages. No ZEC has been sent.">
        Pending · unpaid
      </Badge>
    );
  }
  if (claim.status === "released") {
    if (p?.state === "failed") {
      return (
        <Badge tone="red" title="No ZEC was sent. The amount is available to claim again.">
          Payout failed · not sent
        </Badge>
      );
    }
    return <Badge>Cancelled</Badge>;
  }
  return (
    <Badge tone="green" title={claim.settlement_ref ? `Transaction ${claim.settlement_ref}` : undefined}>
      Paid · payout confirmed
    </Badge>
  );
}
