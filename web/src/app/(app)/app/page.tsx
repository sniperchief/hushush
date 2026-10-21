import Link from "next/link";
import { Legend, StatusDonut, WeeklyBars, WorkerBars } from "@/components/dashboard/charts";
import { CopyButton } from "@/components/dashboard/copy-button";
import { EarningsChart } from "@/components/dashboard/earnings-chart";
import { FaucetButton } from "@/components/dashboard/faucet-button";
import { ActionLink, Card, EmptyBox, Kpi, RangeChip, SummaryTile, requestTime, sumOf } from "@/components/dashboard/sections";
import { ProgressRing, WorkerMetrics } from "@/components/dashboard/worker-live";
import { CompanyForm } from "@/components/employer";
import { IntentQuestion, SwitchIntent } from "@/components/onboarding";
import { PlainShell } from "@/components/plain-shell";
import { AutoRefresh } from "@/components/worker";
import type { SessionUser } from "@/lib/auth";
import { DAY, dailySeries, dayLabel, earnedAt, formatZats, initials, monthBounds, statusCounts, weeklyWithdrawals } from "@/lib/chart-data";
import { companyOf, employerHomeOf, myVestingOf, orgVestingOf, payoutWalletOf, workerStreamsOf } from "@/lib/dashboard";
import type { Company } from "@/lib/company";
import { compactZec, formatTime } from "@/lib/format";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "Overview" };
export const dynamic = "force-dynamic";

export default async function Home() {
  const user = await requirePageUser("/app");
  if (!user.intent) {
    return (
      <PlainShell>
        <IntentQuestion name={user.displayName} />
      </PlainShell>
    );
  }
  if (user.intent === "pay") {
    const company = await companyOf(user.id);
    return company ? <EmployerOverview user={user} company={company} /> : <CompanyStep />;
  }
  return <WorkerOverview user={user} />;
}

function CompanyStep() {
  return (
    <PlainShell>
      <div className="mx-auto flex w-full max-w-[460px] flex-col gap-6 sm:pt-[clamp(8px,4vw,56px)] sm:pb-10">
        <div className="flex flex-col gap-2 sm:gap-2.5 sm:text-center">
          <h1 className="m-0 text-[32px] leading-[1.08] font-semi tracking-[-0.8px] sm:text-4xl sm:leading-[1.1] sm:tracking-[-0.025em]">Name your company</h1>
          <p className="m-0 text-base leading-[1.4] text-bark sm:leading-[1.45]">The people you pay will see this name. Paying people yourself? Use your own name.</p>
        </div>
        <CompanyForm />
        <p className="m-0 text-center">
          <SwitchIntent
            to="get_paid"
            className="min-h-11 cursor-pointer border-0 bg-transparent p-0 text-[15px] text-bark underline underline-offset-[3px] hover:text-espresso disabled:cursor-wait disabled:opacity-60 sm:min-h-10"
          >
            I&apos;m here to get paid instead
          </SwitchIntent>
        </p>
      </div>
    </PlainShell>
  );
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

// ---- employer ----------------------------------------------------------------

async function EmployerOverview({ user, company }: { user: SessionUser; company: Company }) {
  const [home, wallet, vesting] = await Promise.all([employerHomeOf(user.id, company.id), payoutWalletOf(user.id, company.id), orgVestingOf(user.id, company.id)]);
  const now = requestTime();
  const streams = home.streams;
  const waiting = home.offers.filter((o) => o.status === "open");
  const payments = streams.flatMap((s) => s.payouts).sort((a, b) => b.created_at - a.created_at);

  // Metric cards: sums in zatoshis, formatted once.
  const committed = sumOf(streams, (s) => s.total_zats);
  const earned = sumOf(streams, (s) => s.balance.earned_zats);
  const claimed = sumOf(streams, (s) => s.balance.claimed_zats);
  const reserved = sumOf(streams, (s) => s.balance.reserved_zats);
  const from = now - 30 * DAY;
  const earned30 = streams.reduce((a, s) => a + earnedAt(s, now) - earnedAt(s, from), 0n);
  // What the company's payout wallet must cover now.
  const owedNow = sumOf(streams, (s) => s.balance.available_zats);
  const spendable = typeof wallet === "string" ? null : BigInt(wallet.spendable_zats);
  const shortNow = spendable !== null && owedNow > spendable;
  const vested = vesting.reduce((a, v) => a + BigInt(v.balance.vested_zats), 0n);
  const active = streams.filter((s) => s.status === "active").length;

  // Charts.
  const series = dailySeries(streams, from, now).map((p) => ({ t: p.t, earned: Number(p.earned), withdrawn: Number(p.withdrawn) }));
  const first = series[0]!;
  const lastPt = series[series.length - 1]!;
  const weeks = weeklyWithdrawals(streams, now).map((w) => ({
    weekStart: w.weekStart,
    paid: Number(w.paid),
    inFlight: Number(w.inFlight),
    byWorker: w.byWorker.map((b) => ({ name: b.name, paid: Number(b.paid), inFlight: Number(b.inFlight) })),
  }));
  const counts = statusCounts(streams, waiting.length);
  const fz = (n: number) => formatZats(BigInt(n));

  return (
    <>
      <AutoRefresh seconds={30} />
      <div className="flex flex-wrap items-end justify-between gap-4 max-sm:flex-col max-sm:items-stretch">
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-3.5 sm:hidden">
            <span aria-hidden="true" className="flex size-12 flex-none items-center justify-center rounded-[11px] bg-espresso text-base font-semi text-cream">
              {initials(company.name)}
            </span>
            <h1 className="m-0 text-[26px] leading-[1.1] font-semi tracking-[-0.6px]">{company.name}</h1>
          </div>
          <h1 className="m-0 text-[30px] leading-[1.1] font-semi tracking-[-0.6px] max-sm:hidden">Overview</h1>
          <p className="m-0 text-sm leading-[1.4] text-bark sm:text-[15px]">Your company at a glance: what you pay, and the wallet it&apos;s paid from.</p>
        </div>
        <div className="flex items-center gap-2.5 max-sm:flex-col max-sm:items-stretch">
          <span className="mono text-[12.5px] text-bark max-sm:hidden">As of {formatTime(now)}</span>
          <ActionLink href="/app/streams/new">New stream</ActionLink>
        </div>
      </div>

      {/* Metric cards and the payout wallet */}
      <div className="grid grid-cols-2 gap-4 max-sm:grid-cols-1 min-[1180px]:grid-cols-4">
        <Kpi className="max-sm:hidden" label="Committed" value={formatZats(committed)} sub={`Across ${plural(streams.length, "stream")}`} />
        <Kpi
          className="max-sm:hidden"
          label="Earned by workers"
          swatch="#d08a00"
          value={formatZats(earned)}
          sub={
            <>
              <b className="font-semi text-espresso">+{formatZats(earned30)}</b> in the last 30 days
            </>
          }
        />
        <Kpi className="max-sm:hidden" label="Withdrawn" swatch="#7a5cc8" value={formatZats(claimed)} sub={reserved > 0n ? `Plus ${formatZats(reserved)} on its way` : undefined} />
        <section aria-label="Payout wallet" className="flex min-w-0 flex-col gap-2 rounded-xl border border-espresso bg-espresso px-5 py-[18px] text-cream">
          <span className="flex items-center justify-between gap-2 text-[13px] text-oat">
            <span className="flex items-center gap-2">
              Payout wallet
              <span className="rounded-full bg-honey px-[7px] py-px text-[11px] font-semi text-espresso">Testnet</span>
            </span>
            <Link href="/app/wallet" className="text-[12.5px] font-semi text-cream underline decoration-amber decoration-2 underline-offset-4">
              Open →
            </Link>
          </span>
          {typeof wallet === "string" ? (
            <span className="text-sm leading-normal">{wallet}</span>
          ) : (
            <>
              <span className="mono truncate text-[28px] leading-[1.1] font-medium tracking-[-0.6px] text-amber">
                {compactZec(wallet.spendable_zec)} <span className="font-landing text-sm font-normal tracking-normal text-oat">TAZ</span>
              </span>
              <span className={`text-[12.5px] ${shortNow ? "text-honey" : "text-oat"}`}>
                {wallet.pending_zats > 0
                  ? `+${compactZec(wallet.pending_zec)} TAZ settling`
                  : shortNow
                    ? "Can't cover what workers can withdraw now"
                    : wallet.spendable_zats === 0
                      ? "Fund it so workers can withdraw"
                      : "Spendable"}
              </span>
              <div className="mt-1 flex items-center gap-2 rounded-lg border border-cream/14 bg-cream/8 py-1 pr-1 pl-2.5">
                <span className="mono min-w-0 flex-1 truncate text-[11.5px]" title={wallet.funding_address}>
                  {wallet.funding_address.slice(0, 14)}…{wallet.funding_address.slice(-8)}
                </span>
                <CopyButton text={wallet.funding_address} label="Copy funding address" />
              </div>
              <FaucetButton
                compact
                available={wallet.faucet.available}
                amountZec={compactZec(wallet.faucet.amount_zec)}
                nextAt={wallet.faucet.next_at ? formatTime(wallet.faucet.next_at) : null}
                remaining={wallet.faucet.remaining}
              />
            </>
          )}
        </section>
      </div>

      {/* Each section, summarised; the full lists are on their own pages */}
      <div className="grid gap-4 sm:grid-cols-3">
        <SummaryTile
          href="/app/streams"
          icon="streams"
          title="Streams"
          value={plural(active, "active stream")}
          sub={
            waiting.length > 0
              ? `${plural(streams.length, "stream")} in all · ${waiting.length} waiting to join`
              : `${plural(streams.length, "stream")} in all · ${formatZats(committed)} ZEC committed`
          }
        />
        <SummaryTile
          href="/app/vesting"
          icon="vesting"
          title="Vesting"
          value={plural(vesting.length, "agreement")}
          sub={vesting.length > 0 ? `${formatZats(vested)} ZEC vested so far` : "Grant a ZEC allocation that vests over time"}
        />
        <SummaryTile
          href="/app/payments"
          icon="payments"
          title="Payments"
          value={plural(payments.length, "withdrawal")}
          sub={payments[0] ? `Latest ${compactZec(payments[0].amount_zec)} ZEC · ${formatTime(payments[0].created_at)}` : "None yet"}
        />
      </div>

      {streams.length === 0 ? (
        <EmptyBox>
          No worker has joined a stream yet. Create one and send them the worker link.{" "}
          {waiting.length > 0 && (
            <Link href="/app/streams#waiting" className="font-semi text-espresso">
              See who you&apos;re waiting for →
            </Link>
          )}
        </EmptyBox>
      ) : (
        <>
          <div className="grid gap-4 min-[1180px]:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
            <Card
              title="Earned vs withdrawn"
              aside={
                <RangeChip>
                  <span className="max-sm:hidden">
                    {dayLabel(from)} – {dayLabel(now)}
                  </span>
                  <span className="sm:hidden">Last 30 days</span>
                </RangeChip>
              }
            >
              <EarningsChart
                mode="employer"
                points={series}
                from={from}
                to={now}
                now={now}
                ariaLabel={`Total earned by workers rises from ${fz(first.earned)} to ${fz(lastPt.earned)} ZEC over 30 days; total withdrawn goes from ${fz(first.withdrawn)} to ${fz(lastPt.withdrawn)} ZEC`}
              />
            </Card>
            <Card title="Streams by status" aside={<span className="text-[12.5px] text-bark">Now</span>} className="max-sm:hidden">
              <StatusDonut counts={counts} />
            </Card>
          </div>
          <div className="grid gap-4 max-sm:hidden min-[1180px]:grid-cols-2">
            <Card title="Each worker's stream" aside={<span className="text-[12.5px] text-bark">Bar length = stream total</span>}>
              <WorkerBars
                rows={streams.map((s) => ({
                  id: s.id,
                  name: s.worker_name,
                  total: s.total_zats,
                  earned: s.balance.earned_zats,
                  claimed: s.balance.claimed_zats,
                  reserved: s.balance.reserved_zats,
                }))}
              />
              <Legend
                items={[
                  { label: "Withdrawn", color: "#7a5cc8" },
                  { label: "On its way", stripe: true },
                  { label: "Earned, not withdrawn", color: "#d08a00" },
                  { label: "Still to earn", color: "#efece5" },
                ]}
              />
            </Card>
            <Card title="Withdrawals per week" aside={<RangeChip>Last 5 weeks</RangeChip>}>
              <WeeklyBars weeks={weeks} />
            </Card>
          </div>
        </>
      )}
    </>
  );
}

// ---- worker ------------------------------------------------------------------

async function WorkerOverview({ user }: { user: SessionUser }) {
  const [streams, vesting] = await Promise.all([workerStreamsOf(user.id), myVestingOf(user.id)]);
  const now = requestTime();
  const missing = streams.filter((s) => !s.address_locked).length;
  const payouts = streams.flatMap((s) => s.payouts).sort((a, b) => b.created_at - a.created_at);
  const lastConfirmed = payouts.find((p) => p.state === "confirmed" && p.resolved_at != null);
  const lastPaid = lastConfirmed ? `${dayLabel(lastConfirmed.resolved_at!)} ${new Date(lastConfirmed.resolved_at! * 1000).getUTCFullYear()}` : null;
  const vestingAvailable = vesting.reduce((a, v) => a + BigInt(v.balance.available_zats), 0n);

  const header = (
    <div className="flex flex-wrap items-end justify-between gap-4">
      <div className="flex flex-col gap-1.5">
        <h1 className="m-0 text-[30px] leading-[1.1] font-semi tracking-[-0.6px]">Hi {user.displayName}</h1>
        <p className="m-0 text-base text-bark sm:text-[15px]">What you&apos;re earning, live.</p>
        <span className="min-[900px]:hidden">
          <SwitchIntent to="pay" className="min-h-9 cursor-pointer border-0 bg-transparent p-0 text-sm text-bark underline underline-offset-[3px] disabled:opacity-60">
            Start paying people →
          </SwitchIntent>
        </span>
      </div>
      {streams.length > 0 && (
        <span className="inline-flex items-center gap-2 text-[13px] text-bark max-sm:hidden">
          <span aria-hidden="true" className="inline-block size-[7px] flex-none animate-live rounded-full bg-amber motion-reduce:animate-none" />
          Ticking every second
        </span>
      )}
    </div>
  );

  const tiles = (
    <div className="grid gap-4 sm:grid-cols-3">
      <SummaryTile
        href="/app/streams"
        icon="streams"
        title="My streams"
        value={plural(streams.length, "stream")}
        sub={missing > 0 ? `${plural(missing, "stream")} still need a payout address` : streams.length > 0 ? "Payout addresses locked" : "None yet"}
      />
      <SummaryTile
        href="/app/withdrawals"
        icon="payments"
        title="Withdrawals"
        value={plural(payouts.length, "withdrawal")}
        sub={lastPaid ? `Last paid ${lastPaid}` : "None yet"}
      />
      <SummaryTile
        href="/app/vesting"
        icon="vesting"
        title="Vesting"
        value={plural(vesting.length, "agreement")}
        sub={vesting.length > 0 ? `${formatZats(vestingAvailable)} ZEC available to withdraw` : "None yet"}
      />
    </div>
  );

  if (streams.length === 0) {
    return (
      <>
        <AutoRefresh seconds={15} />
        {header}
        {tiles}
        {vesting.length === 0 && (
          <EmptyBox>Nobody is paying you through hushush yet. When someone does, they&apos;ll send you a link. Open it to start receiving pay.</EmptyBox>
        )}
      </>
    );
  }

  // Chart range: the month of the earliest active stream's start (through this month).
  const active = streams.filter((s) => s.status === "active");
  const earliest = Math.min(...(active.length ? active : streams).map((s) => s.start_at));
  let { from, to } = monthBounds(earliest);
  if (from > now) from = monthBounds(now).from;
  if (to <= now) to = monthBounds(now).to;
  const past = dailySeries(streams, from, now).map((p) => ({ t: p.t, earned: Number(p.earned), withdrawn: Number(p.withdrawn) }));
  const projection = dailySeries(streams, now, to).map((p) => ({ t: p.t, earned: Number(p.earned) }));
  const nowPt = past[past.length - 1]!;
  const projEnd = projection[projection.length - 1]!;
  const ring = streams.reduce((a, b) => (b.balance.available_zats > a.balance.available_zats ? b : a), streams[0]!);
  const asOfKey = Math.max(...streams.map((s) => s.balance.as_of));

  return (
    <>
      <AutoRefresh seconds={15} />
      {header}
      <div className="max-sm:hidden">
        <WorkerMetrics key={asOfKey} streams={streams} lastPaid={lastPaid} />
      </div>
      <div className="grid gap-4 min-[1180px]:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
        <Card
          title="Your earnings"
          aside={
            <RangeChip>
              {dayLabel(from)} – {dayLabel(to - 1)}
            </RangeChip>
          }
        >
          <EarningsChart
            mode="worker"
            points={past}
            projection={projection}
            from={from}
            to={to}
            now={now}
            ariaLabel={`Earned rises from ${formatZats(BigInt(past[0]!.earned))} on ${dayLabel(from)} to ${formatZats(BigInt(nowPt.earned))} ZEC today and is projected to reach ${formatZats(BigInt(projEnd.earned))} ZEC by ${dayLabel(to - 1)}; withdrawn ${formatZats(BigInt(nowPt.withdrawn))} ZEC`}
          />
        </Card>
        <Card title={ring.organization_name} aside={<span className="text-[12.5px] whitespace-nowrap text-bark">{compactZec(ring.total_zec)} ZEC stream</span>} className="max-sm:hidden">
          <ProgressRing key={ring.balance.as_of} s={ring} />
        </Card>
      </div>
      {tiles}
    </>
  );
}
