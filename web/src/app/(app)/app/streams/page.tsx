import Link from "next/link";
import { redirect } from "next/navigation";
import { Avatar } from "@/components/dashboard/avatar";
import { ActionLink, Card, EmptyBox, PageHeader, StreamTag, WorkerStreamCard, sumOf } from "@/components/dashboard/sections";
import { OfferActions } from "@/components/employer";
import { AutoRefresh } from "@/components/worker";
import type { SessionUser } from "@/lib/auth";
import { formatZats } from "@/lib/chart-data";
import type { Company } from "@/lib/company";
import { companyOf, employerHomeOf, workerStreamsOf } from "@/lib/dashboard";
import { compactZec, formatTime } from "@/lib/format";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "Streams" };
export const dynamic = "force-dynamic";

export default async function StreamsPage() {
  const user = await requirePageUser("/app/streams");
  if (!user.intent) redirect("/app");
  if (user.intent === "pay") {
    const company = await companyOf(user.id);
    if (!company) redirect("/app");
    return <EmployerStreams user={user} company={company} />;
  }
  return <WorkerStreams user={user} />;
}

async function EmployerStreams({ user, company }: { user: SessionUser; company: Company }) {
  const home = await employerHomeOf(user.id, company.id);
  const streams = home.streams;
  const waiting = home.offers.filter((o) => o.status === "open");
  const committed = sumOf(streams, (s) => s.total_zats);

  return (
    <>
      <AutoRefresh seconds={30} />
      <PageHeader
        title="Streams"
        lead="Pay for ongoing work: each worker earns their total evenly, by the second, and withdraws at every 20% milestone."
        action={<ActionLink href="/app/streams/new">New stream</ActionLink>}
      />

      <Card
        id="streams"
        title="All streams"
        icon="streams"
        count={streams.length}
        aside={
          streams.length > 0 && (
            <span className="text-[12.5px] text-bark">
              {formatZats(committed)} ZEC committed · updates every 30 seconds
            </span>
          )
        }
      >
        {streams.length === 0 ? (
          <EmptyBox>No worker has joined a stream yet. Create one and send them the worker link.</EmptyBox>
        ) : (
          <>
            <div className="overflow-x-auto max-sm:hidden">
              <table className="w-full min-w-[620px] border-collapse text-sm">
                <thead>
                  <tr className="border-b border-linen text-left text-xs font-semi text-bark">
                    <th className="pr-3 pb-3 font-semi">Worker</th>
                    <th className="px-3 pb-3 text-right font-semi">Total</th>
                    <th className="px-3 pb-3 text-right font-semi">Earned</th>
                    <th className="px-3 pb-3 text-right font-semi">Withdrawn</th>
                    <th className="px-3 pb-3 font-semi">Status</th>
                    <th className="pb-3 pl-3 font-semi">Ends</th>
                  </tr>
                </thead>
                <tbody>
                  {streams.map((s) => {
                    const ended = s.status === "ended";
                    return (
                      <tr key={s.id} className={`border-b border-mist last:border-0 ${ended ? "text-bark" : ""}`}>
                        <td className="py-3.5 pr-3 whitespace-nowrap">
                          <Link href={`/app/streams/${s.id}`} className={`flex items-center gap-2.5 font-semi no-underline hover:underline ${ended ? "text-bark" : "text-espresso"}`}>
                            <Avatar name={s.worker_name} />
                            {s.worker_name}
                          </Link>
                        </td>
                        <td className="mono px-3 py-3.5 text-right">{compactZec(s.total_zec)}</td>
                        <td className="mono px-3 py-3.5 text-right">{compactZec(s.balance.earned_zec)}</td>
                        <td className="mono px-3 py-3.5 text-right">{compactZec(s.balance.claimed_zec)}</td>
                        <td className="px-3 py-3.5">
                          <StreamTag s={s} />
                        </td>
                        <td className="py-3.5 pl-3 text-[13px] whitespace-nowrap text-bark">{formatTime(s.ended_at ?? s.end_at)}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
            <div className="flex flex-col sm:hidden">
              {streams.map((s) => {
                const pct = (v: number) => `${s.total_zats > 0 ? (v / s.total_zats) * 100 : 0}%`;
                return (
                  <Link key={s.id} href={`/app/streams/${s.id}`} className="flex flex-col gap-2.5 border-b border-mist py-3.5 text-espresso no-underline first:pt-0 last:border-0 last:pb-0">
                    <div className="flex items-center justify-between gap-2.5">
                      <span className="flex min-w-0 items-center gap-2.5 font-semi">
                        <Avatar name={s.worker_name} />
                        <span className="truncate">{s.worker_name}</span>
                      </span>
                      <StreamTag s={s} />
                    </div>
                    <div className="relative h-2 overflow-hidden rounded bg-mist">
                      <span className="absolute inset-y-0 left-0 bg-earned" style={{ width: pct(s.balance.earned_zats) }} />
                      <span className="absolute inset-y-0 left-0 bg-withdrawn" style={{ width: pct(s.balance.claimed_zats) }} />
                    </div>
                    <div className="flex justify-between text-[12.5px] text-bark">
                      <span>
                        Earned <span className="mono text-espresso">{compactZec(s.balance.earned_zec)}</span> of <span className="mono">{compactZec(s.total_zec)}</span>
                      </span>
                      <span>
                        Withdrawn <span className="mono text-espresso">{compactZec(s.balance.claimed_zec)}</span>
                      </span>
                    </div>
                  </Link>
                );
              })}
            </div>
          </>
        )}
      </Card>

      <Card id="waiting" title="Waiting to join" icon="waiting" count={waiting.length}>
        {waiting.length === 0 ? (
          <p className="m-0 text-sm leading-normal text-bark">Nobody. When you create a stream, it waits here until the worker opens their link and joins.</p>
        ) : (
          <ul className="m-0 flex list-none flex-col divide-y divide-mist p-0">
            {waiting.map((o) => (
              <li key={o.id} className="flex flex-col gap-3 py-4 first:pt-0 last:pb-0">
                <div className="flex items-center justify-between gap-2">
                  <span className="flex min-w-0 items-center gap-2.5 font-semi">
                    <Avatar name={o.worker_name} dashed />
                    <span className="truncate">{o.worker_name}</span>
                  </span>
                  <span className="mono">{compactZec(o.total_zec)} ZEC</span>
                </div>
                <p className="m-0 text-[13px] leading-normal text-bark">
                  {formatTime(o.start_at)} → {formatTime(o.end_at)}. Earnings count from the start date even if they join later.
                </p>
                <OfferActions offerId={o.id} />
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}

async function WorkerStreams({ user }: { user: SessionUser }) {
  const streams = await workerStreamsOf(user.id);
  const missing = streams.filter((s) => !s.address_locked).length;
  return (
    <>
      <AutoRefresh seconds={15} />
      <PageHeader
        title="My streams"
        lead={
          missing > 0
            ? `Set a payout address on ${missing === 1 ? "one stream" : `${missing} streams`} to withdraw from ${missing === 1 ? "it" : "them"}. Each address is locked for the life of its stream.`
            : "Streams paying you, live. You can withdraw at each 20% milestone, and whatever is left once a stream ends."
        }
      />
      <div id="addresses" className="scroll-mt-4" />
      {streams.length === 0 ? (
        <EmptyBox>Nobody is paying you through hushush yet. When someone does, they&apos;ll send you a link. Open it to start receiving pay.</EmptyBox>
      ) : (
        <div className="grid items-start gap-4 min-[1180px]:grid-cols-2">
          {streams.map((s) => (
            <WorkerStreamCard key={s.id} s={s} />
          ))}
        </div>
      )}
    </>
  );
}
