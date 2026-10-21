import { PageHeader } from "@/components/dashboard/sections";
import { PayoutList } from "@/components/payouts";
import { Badge, Card, CardTitle, Stat } from "@/components/ui";
import { VestingActions, VestingWithdrawPanel } from "@/components/vesting";
import { AutoRefresh } from "@/components/worker";
import { compactZec, formatTime } from "@/lib/format";
import { orNotFound } from "@/lib/page-data";
import { payroll, type VestingEvent } from "@/lib/payroll";
import { requirePageUser } from "@/lib/session";
import { CANCELLATION_RULE, pct, phaseLabel, phaseTone, scheduleSentences, zecOf } from "@/lib/vesting";

export const dynamic = "force-dynamic";

type Props = { params: Promise<{ agreementId: string }> };

function eventLabel(e: VestingEvent): string {
  switch (e.kind) {
    case "created":
      return "Draft created";
    case "activated":
      return "Activated";
    case "cancelled":
      return `Cancelled: vesting stopped with ${zecOf(BigInt(e.vested_zats_after))} ZEC vested`;
  }
}

/** One agreement, for its employee (withdraw) or the company's managers (manage). */
export default async function VestingPage({ params }: Props) {
  const { agreementId } = await params;
  const user = await requirePageUser(`/app/vesting/${agreementId}`);
  const d = await orNotFound(payroll.vesting(user.id, agreementId));
  const v = d.agreement;
  const b = v.balance;
  const employee = d.viewer === "employee";
  const inFlight = d.payouts.some((p) => p.state === "reserved" || p.state === "submitted" || p.state === "requires_reconciliation");
  const cancelled = v.status === "cancelled";

  return (
    <>
      <AutoRefresh seconds={inFlight ? 5 : 30} />
      <PageHeader
        back={{ href: "/app/vesting", label: "Vesting" }}
        title={`${employee ? v.organization_name : v.employee_display_name} · ${compactZec(v.total_zec)} ZEC`}
        tag={<Badge tone={phaseTone(v.phase)}>{phaseLabel(v.phase)}</Badge>}
        lead={
          <>
            Vesting agreement · {formatTime(v.start_at)} → {formatTime(v.end_at)}
            {v.cancelled_at && <> · cancelled {formatTime(v.cancelled_at)}</>}
          </>
        }
        action={!employee && <VestingActions v={v} />}
      />

      <Card>
        <CardTitle icon="vesting" action={<span className="text-[12.5px] text-bark">as of {formatTime(b.as_of)}</span>}>Balances</CardTitle>
        <div
          className="relative mb-4 h-2.5 overflow-hidden rounded bg-mist"
          role="img"
          aria-label={`${compactZec(b.vested_zec)} of ${compactZec(b.total_zec)} ZEC vested; ${compactZec(b.paid_zec)} ZEC paid`}
        >
          <div className="absolute inset-y-0 left-0 bg-earned" style={{ width: `${pct(b.vested_zats, b.total_zats)}%` }} />
          <div className="absolute inset-y-0 left-0 bg-withdrawn" style={{ width: `${pct(b.paid_zats, b.total_zats)}%` }} />
          <div className="stripe-held absolute inset-y-0" style={{ left: `${pct(b.paid_zats, b.total_zats)}%`, width: `${pct(b.reserved_zats, b.total_zats)}%` }} />
        </div>
        <div className="grid gap-3 sm:grid-cols-3 lg:grid-cols-5">
          <Stat label="Vested" value={compactZec(b.vested_zec)} hint={`of ${compactZec(b.total_zec)} ZEC`} />
          <Stat label={cancelled ? "Forfeited" : "Unvested"} value={compactZec(b.unvested_zec)} hint={cancelled ? "Will never vest" : undefined} />
          <Stat label="Paid" value={compactZec(b.paid_zec)} hint="Confirmed on chain" />
          <Stat label="On its way" value={compactZec(b.reserved_zec)} hint="Withdrawal in progress, not yet paid" />
          <Stat label="Available" value={compactZec(b.available_zec)} emphasis={employee && b.available_zats > 0} />
        </div>
      </Card>

      {employee && (
        <Card>
          <CardTitle icon="payments">Withdraw</CardTitle>
          <VestingWithdrawPanel d={d} />
          {d.address && (
            <p className="m-0 mt-4 text-[12.5px] leading-[1.45] text-bark">
              Locked payout address: <span className="mono break-all">{d.address}</span>
            </p>
          )}
          <p className="m-0 mt-2 text-[12.5px] leading-[1.45] text-bark">
            Paid as a shielded Zcash transaction: the amount and the recipient aren&apos;t visible on chain. That a transaction happened, its
            timing and its fee are.
          </p>
        </Card>
      )}

      <Card>
        <CardTitle icon="overview">Schedule</CardTitle>
        <div className="flex flex-col gap-2 text-sm leading-relaxed [&>p]:m-0">
          {scheduleSentences(v).map((l) => (
            <p key={l}>{l}</p>
          ))}
          <p className="text-bark">{CANCELLATION_RULE}</p>
          {v.status === "draft" && (
            <p className="text-bark">
              This is a draft: nothing vests and {v.employee_display_name} can&apos;t see it until it is activated, which must happen by{" "}
              {formatTime(v.start_at)}.
            </p>
          )}
          {!employee && (
            <p className="text-bark">
              Payout address: {d.address_locked ? "set and locked by the employee" : "not set yet"}. Only the employee can see it.
            </p>
          )}
        </div>
      </Card>

      <Card>
        <CardTitle icon="payments">Withdrawals</CardTitle>
        <PayoutList payouts={d.payouts} empty={employee ? "No withdrawals yet." : "The employee hasn't withdrawn yet."} />
      </Card>

      <Card>
        <CardTitle icon="waiting">History</CardTitle>
        <ul className="m-0 list-none divide-y divide-mist p-0 text-sm">
          {d.events.map((e) => (
            <li key={`${e.kind}-${e.occurred_at}`} className="flex flex-wrap justify-between gap-2 py-2">
              <span>{eventLabel(e)}</span>
              <span className="text-[12.5px] text-bark">{formatTime(e.occurred_at)}</span>
            </li>
          ))}
        </ul>
      </Card>
    </>
  );
}
