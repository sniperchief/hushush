import { redirect } from "next/navigation";
import { Card, EmptyBox, PageHeader } from "@/components/dashboard/sections";
import { PayoutBadge, PayoutDetail } from "@/components/payouts";
import { AutoRefresh } from "@/components/worker";
import { workerStreamsOf } from "@/lib/dashboard";
import { compactZec } from "@/lib/format";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "Withdrawals" };
export const dynamic = "force-dynamic";

/** The worker's withdrawals from all their streams (employers have /app/payments). */
export default async function WithdrawalsPage() {
  const user = await requirePageUser("/app/withdrawals");
  if (user.intent === "pay") redirect("/app/payments");
  if (!user.intent) redirect("/app");
  const streams = await workerStreamsOf(user.id);
  const payouts = streams.flatMap((s) => s.payouts.map((p) => ({ p, org: s.organization_name }))).sort((a, b) => b.p.created_at - a.p.created_at);

  return (
    <>
      <AutoRefresh seconds={15} />
      <PageHeader
        title="Withdrawals"
        lead="What you've withdrawn from your streams, newest first. A withdrawal shows Paid only once it's confirmed on chain. Vesting withdrawals are listed on each agreement's page."
      />
      <Card id="withdrawals" title="Stream withdrawals" icon="payments" count={payouts.length}>
        {payouts.length === 0 ? (
          <EmptyBox>No withdrawals yet.</EmptyBox>
        ) : (
          <ul className="m-0 flex list-none flex-col p-0">
            {payouts.map(({ p, org }) => (
              <li key={p.id} className="flex items-start justify-between gap-3 border-b border-mist py-3 first:pt-0 last:border-0 last:pb-0">
                <div className="min-w-0">
                  <div className="mono font-semi">{compactZec(p.amount_zec)} ZEC</div>
                  <div className="mt-[3px] text-[12.5px] leading-[1.45] text-bark">
                    {org} · <PayoutDetail p={p} />
                  </div>
                </div>
                <PayoutBadge p={p} />
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}
