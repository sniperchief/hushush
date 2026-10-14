import Link from "next/link";
import { EndStreamButton } from "@/components/employer";
import { PayoutList } from "@/components/payouts";
import { Badge, Card, CardTitle, Stat } from "@/components/ui";
import { AddressForm, AutoRefresh, WorkerStreamPanel } from "@/components/worker";
import { compactZec, formatTime } from "@/lib/format";
import { orNotFound } from "@/lib/page-data";
import { payroll } from "@/lib/payroll";
import { requirePageUser } from "@/lib/session";

export const dynamic = "force-dynamic";

type Props = { params: Promise<{ streamId: string }> };

export default async function StreamPage({ params }: Props) {
  const { streamId } = await params;
  const user = await requirePageUser(`/app/streams/${streamId}`);
  const s = await orNotFound(payroll.flowStream(user.id, streamId));
  const inFlight = s.payouts.some((p) => p.state === "reserved" || p.state === "submitted" || p.state === "requires_reconciliation");
  const ended = s.status === "ended";
  const pct = Math.floor((s.balance.earned_zats * 100) / s.total_zats);

  const header = (
    <div>
      <Link href="/app" className="text-sm text-ink-soft hover:underline">
        ← Home
      </Link>
      <div className="mt-2 flex flex-wrap items-center gap-3">
        <h1 className="text-2xl font-semibold tracking-tight">
          {user.kind === "worker" ? s.organization_name : s.worker_name} · {compactZec(s.total_zec)} ZEC
        </h1>
        {ended ? <Badge>Ended</Badge> : <Badge tone="green">● Active</Badge>}
      </div>
      <p className="mt-1 text-sm text-ink-soft">
        {formatTime(s.start_at)} → {formatTime(s.end_at)}
        {s.ended_at && s.ended_at < s.end_at && <> · ended early {formatTime(s.ended_at)}</>}
      </p>
    </div>
  );

  if (user.kind === "worker") {
    return (
      <div className="space-y-6">
        <AutoRefresh seconds={inFlight ? 5 : 15} />
        {header}
        <Card>
          <WorkerStreamPanel s={s} />
        </Card>
        <Card>
          <CardTitle>Payout address</CardTitle>
          {s.address ? (
            <div className="space-y-2">
              <div className="flex items-start gap-2 rounded-xl bg-muted px-3 py-2">
                <span aria-label="Locked">🔒</span>
                <span className="break-all font-mono text-xs">{s.address}</span>
              </div>
              <p className="text-xs text-ink-soft">
                Locked for the life of this stream. Every withdrawal goes here. To be paid somewhere else, your employer would end this stream and
                start a new one.
              </p>
            </div>
          ) : (
            <AddressForm streamId={s.id} />
          )}
        </Card>
        <Card>
          <CardTitle>Withdrawals</CardTitle>
          <PayoutList payouts={s.payouts} />
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <AutoRefresh seconds={inFlight ? 5 : 30} />
      <div className="flex flex-wrap items-start justify-between gap-4">
        {header}
        {!ended && <EndStreamButton streamId={s.id} />}
      </div>
      <Card>
        <CardTitle action={<span className="text-xs text-ink-soft">as of {formatTime(s.balance.as_of)}</span>}>Progress · {pct}%</CardTitle>
        <div className="grid gap-3 sm:grid-cols-4">
          <Stat label="Earned" value={compactZec(s.balance.earned_zec)} />
          <Stat label="Withdrawn (paid)" value={compactZec(s.balance.claimed_zec)} />
          <Stat label="On its way" value={compactZec(s.balance.reserved_zec)} />
          <Stat label="Not yet withdrawn" value={compactZec(s.balance.available_zec)} />
        </div>
        <p className="mt-4 text-xs text-ink-soft">
          Payout address: {s.address_locked ? "set and locked by the worker" : "not set yet"}. Only the worker can see it.
        </p>
      </Card>
      <Card>
        <CardTitle>Payouts</CardTitle>
        <PayoutList payouts={s.payouts} empty="The worker hasn't withdrawn yet." />
      </Card>
    </div>
  );
}
