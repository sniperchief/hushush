import Link from "next/link";
import { AutoRefresh, ClaimForm, RefreshButton, ReleaseClaimButton, StreamActions } from "@/components/forms/stream-forms";
import { ClaimStatusBadge, StreamStatusBadge } from "@/components/status";
import { Alert, Card, CardTitle, Empty, Stat } from "@/components/ui";
import { compactZec, formatClock, formatTime } from "@/lib/format";
import { orNotFound } from "@/lib/page-data";
import { payroll } from "@/lib/payroll";
import { periodLabel } from "@/lib/periods";
import { requirePageUser } from "@/lib/session";

export const dynamic = "force-dynamic";

type Props = { params: Promise<{ streamId: string }> };

const EVENT_LABEL = { created: "Stream created", paused: "Paused", resumed: "Resumed", ended: "Ended" } as const;

export default async function StreamPage({ params }: Props) {
  const { streamId } = await params;
  const user = await requirePageUser(`/app/streams/${streamId}`);
  const [{ stream: s, events, claims }, orgs] = await Promise.all([
    orNotFound(payroll.stream(user.id, streamId)),
    payroll.listOrgs(user.id),
  ]);
  const role = orgs.find((o) => o.organization_id === s.organization_id)?.role;
  const isManager = role === "owner" || role === "admin";
  const isRecipient = s.recipient_user_id === user.id;
  const b = s.balance;
  const names = new Map([[s.recipient_user_id, s.recipient_display_name]]);

  return (
    <div className="space-y-6">
      <AutoRefresh />
      <div>
        <Link href={`/app/orgs/${s.organization_id}`} className="text-sm text-ink-soft hover:underline">← {s.organization_name}</Link>
        <div className="mt-2 flex flex-wrap items-center justify-between gap-4">
          <div>
            <div className="flex flex-wrap items-center gap-3">
              <h1 className="text-2xl font-semibold tracking-tight">{isRecipient ? `Your stream from ${s.organization_name}` : s.recipient_display_name}</h1>
              <StreamStatusBadge status={s.status} />
            </div>
            <p className="mt-1 text-sm text-ink-soft">
              <span className="font-mono text-ink">{compactZec(s.rate_zec)} ZEC</span> {periodLabel(s.rate_period_secs)} · started {formatTime(s.start_at)}
              {s.end_at ? ` · ends ${formatTime(s.end_at)}` : " · no end date"}
            </p>
          </div>
          {isManager && <StreamActions streamId={s.id} status={s.status} />}
        </div>
      </div>

      <Card>
        <CardTitle
          action={
            <div className="flex items-center gap-3">
              <span className="text-xs text-ink-soft">Computed by the server as of {formatClock(b.as_of)}</span>
              <RefreshButton />
            </div>
          }
        >
          Balances
        </CardTitle>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <Stat label="Earned" value={b.earned_zec} hint="Accrued over active time" />
          <Stat label="Reserved" value={b.reserved_zec} hint="Pending claims, not yet paid" />
          <Stat label="Claimed" value={b.claimed_zec} hint="Confirmed payouts only" />
          <Stat label="Available" value={b.available_zec} hint="Earned − reserved − claimed" emphasis />
        </div>
        {s.status === "paused" && <div className="mt-4"><Alert tone="amber">Accrual is paused. Wages earned so far are kept.</Alert></div>}
        {s.status === "ended" && <div className="mt-4"><Alert tone="neutral">This stream has ended. Earned wages remain claimable.</Alert></div>}
      </Card>

      <div className="grid gap-6 lg:grid-cols-[1fr_1fr]">
        {isRecipient && (
          <Card>
            <CardTitle>Request a claim</CardTitle>
            <ClaimForm streamId={s.id} availableZec={b.available_zec} availableZats={b.available_zats} />
          </Card>
        )}

        <Card className={isRecipient ? "" : "lg:col-span-2"}>
          <CardTitle>Claims</CardTitle>
          {claims.length === 0 ? (
            <Empty>No claims yet.</Empty>
          ) : (
            <ul className="divide-y divide-line">
              {[...claims].reverse().map((c) => (
                <li key={c.id} className="flex flex-wrap items-center justify-between gap-3 py-3">
                  <div>
                    <div className="font-mono font-medium tabular-nums">{c.amount_zec} ZEC</div>
                    <div className="text-xs text-ink-soft">Requested {formatTime(c.created_at)}</div>
                  </div>
                  <div className="flex items-center gap-2">
                    <ClaimStatusBadge claim={c} />
                    {c.status === "reserved" && (isRecipient || isManager) && <ReleaseClaimButton claimId={c.id} />}
                  </div>
                </li>
              ))}
            </ul>
          )}
          <p className="mt-4 text-xs text-ink-soft">
            Payouts aren&apos;t enabled yet. &ldquo;Pending · unpaid&rdquo; means the amount is reserved for you and no ZEC has been sent.
          </p>
        </Card>
      </div>

      <Card>
        <CardTitle>History</CardTitle>
        <ol className="relative space-y-4 border-l border-line pl-5">
          {events.map((e, i) => (
            <li key={i}>
              <span className="absolute -left-[5px] mt-1.5 h-2.5 w-2.5 rounded-full bg-accent" aria-hidden />
              <div className="text-sm font-medium">{EVENT_LABEL[e.kind]}</div>
              <div className="text-xs text-ink-soft">
                {formatTime(e.occurred_at)}
                {names.get(e.actor_user_id) ? ` · by ${names.get(e.actor_user_id)}` : e.actor_user_id === user.id ? " · by you" : ""}
                {` · ${Math.floor(e.banked_active_secs_after / 3600)}h ${Math.floor((e.banked_active_secs_after % 3600) / 60)}m active`}
              </div>
            </li>
          ))}
        </ol>
      </Card>
    </div>
  );
}
