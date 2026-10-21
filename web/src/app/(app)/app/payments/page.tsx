import Link from "next/link";
import { redirect } from "next/navigation";
import { Avatar } from "@/components/dashboard/avatar";
import { Card, EmptyBox, PageHeader } from "@/components/dashboard/sections";
import { PayoutBadge, Txid } from "@/components/payouts";
import { AutoRefresh } from "@/components/worker";
import { companyOf, employerHomeOf } from "@/lib/dashboard";
import { compactZec, formatTime } from "@/lib/format";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "Payments" };
export const dynamic = "force-dynamic";

/** Withdrawals from the company's streams (employers; workers have /app/withdrawals). */
export default async function PaymentsPage() {
  const user = await requirePageUser("/app/payments");
  if (user.intent === "get_paid") redirect("/app/withdrawals");
  const company = user.intent === "pay" ? await companyOf(user.id) : null;
  if (!company) redirect("/app");
  const home = await employerHomeOf(user.id, company.id);
  const payments = home.streams
    .flatMap((s) => s.payouts.map((p) => ({ p, worker: s.worker_name, streamId: s.id })))
    .sort((a, b) => b.p.created_at - a.p.created_at);

  return (
    <>
      <AutoRefresh seconds={30} />
      <PageHeader
        title="Payments"
        lead="Every withdrawal from your streams, newest first. Each is a shielded Zcash transaction, paid from your payout wallet and marked Paid only once it's confirmed on chain. Vesting withdrawals are listed on each agreement's page."
      />
      <Card id="payments" title="Stream withdrawals" icon="payments" count={payments.length}>
        {payments.length === 0 ? (
          <EmptyBox>No withdrawals yet.</EmptyBox>
        ) : (
          <ul className="m-0 flex list-none flex-col p-0">
            {payments.map(({ p, worker, streamId }) => (
              <li key={p.id} className="flex items-center justify-between gap-3 border-b border-mist py-3 first:pt-0 last:border-0 last:pb-0">
                <div className="flex min-w-0 items-center gap-3">
                  <span className="max-sm:hidden">
                    <Avatar name={worker} />
                  </span>
                  <div className="min-w-0">
                    <div className="text-sm">
                      <Link href={`/app/streams/${streamId}`} className="font-semi text-espresso no-underline hover:underline">
                        {worker}
                      </Link>{" "}
                      <span className="mono">{compactZec(p.amount_zec)} ZEC</span>
                    </div>
                    <div className="mt-[3px] text-xs text-bark sm:text-[12.5px]">
                      {formatTime(p.created_at)}
                      {p.txid && (
                        <span className="max-sm:hidden">
                          {" · "}
                          <Txid txid={p.txid} />
                        </span>
                      )}
                    </div>
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
