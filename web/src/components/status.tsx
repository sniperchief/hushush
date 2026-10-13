import type { ClaimView, StreamStatus } from "@/lib/payroll";
import { Badge } from "./ui";

export function StreamStatusBadge({ status }: { status: StreamStatus }) {
  if (status === "active") return <Badge tone="green">● Active</Badge>;
  if (status === "paused") return <Badge tone="amber">❚❚ Paused</Badge>;
  return <Badge>Ended</Badge>;
}

/**
 * Claim status labels. A reserved claim is ALWAYS shown as pending and unpaid;
 * "Paid" appears only for a settled claim, which only a confirmed payout can
 * produce (not possible in this phase).
 */
export function ClaimStatusBadge({ claim }: { claim: Pick<ClaimView, "status" | "settlement_ref"> }) {
  if (claim.status === "reserved") {
    return (
      <Badge tone="amber" title="Reserved against earned wages. No ZEC has been sent.">
        Pending · unpaid
      </Badge>
    );
  }
  if (claim.status === "released") return <Badge>Cancelled</Badge>;
  return (
    <Badge tone="green" title={claim.settlement_ref ? `Payout ${claim.settlement_ref}` : undefined}>
      Paid · payout confirmed
    </Badge>
  );
}
