import { Icon } from "@/components/dashboard/icons";
import { PageHeader } from "@/components/dashboard/sections";
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
  // The same account can pay on some streams and be paid on others.
  const beingPaid = s.recipient_user_id === user.id;

  const headerProps = {
    back: { href: "/app/streams", label: beingPaid ? "My streams" : "Streams" },
    title: `${beingPaid ? s.organization_name : s.worker_name} · ${compactZec(s.total_zec)} ZEC`,
    tag: ended ? <Badge>Ended</Badge> : <Badge tone="green">Active</Badge>,
    lead: (
      <>
        {formatTime(s.start_at)} → {formatTime(s.end_at)}
        {s.ended_at && s.ended_at < s.end_at && <> · ended early {formatTime(s.ended_at)}</>}
      </>
    ),
  };

  if (beingPaid) {
    return (
      <>
        <AutoRefresh seconds={inFlight ? 5 : 15} />
        <PageHeader {...headerProps} />
        <Card>
          <WorkerStreamPanel s={s} />
        </Card>
        <Card>
          <CardTitle icon="lock">Payout address</CardTitle>
          {s.address ? (
            <div className="flex flex-col gap-2">
              <div className="flex items-start gap-2.5 rounded-lg border border-linen bg-cream px-3.5 py-3">
                <span aria-label="Locked" className="mt-0.5 flex-none text-bark">
                  <Icon name="lock" size={14} strokeWidth={2.2} />
                </span>
                <span className="mono text-xs break-all">{s.address}</span>
              </div>
              <p className="m-0 text-[12.5px] leading-[1.45] text-bark">
                Locked for the life of this stream. Every withdrawal goes here. To be paid somewhere else, your employer would end this stream and
                start a new one.
              </p>
            </div>
          ) : (
            <AddressForm streamId={s.id} />
          )}
        </Card>
        <Card>
          <CardTitle icon="payments">Withdrawals</CardTitle>
          <PayoutList payouts={s.payouts} />
        </Card>
      </>
    );
  }

  return (
    <>
      <AutoRefresh seconds={inFlight ? 5 : 30} />
      <PageHeader {...headerProps} action={!ended && <EndStreamButton streamId={s.id} />} />
      <Card>
        <CardTitle icon="overview" action={<span className="text-[12.5px] text-bark">as of {formatTime(s.balance.as_of)}</span>}>
          Progress · {pct}%
        </CardTitle>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <Stat label="Earned" value={compactZec(s.balance.earned_zec)} />
          <Stat label="Withdrawn (paid)" value={compactZec(s.balance.claimed_zec)} />
          <Stat label="On its way" value={compactZec(s.balance.reserved_zec)} />
          <Stat label="Not yet withdrawn" value={compactZec(s.balance.available_zec)} />
        </div>
        <p className="m-0 mt-4 text-[12.5px] leading-[1.45] text-bark">
          Payout address: {s.address_locked ? "set and locked by the worker" : "not set yet"}. Only the worker can see it.
        </p>
      </Card>
      <Card>
        <CardTitle icon="payments">Payouts</CardTitle>
        <PayoutList payouts={s.payouts} empty="The worker hasn't withdrawn yet." />
      </Card>
    </>
  );
}
