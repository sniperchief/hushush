import Link from "next/link";
import type { ReactNode } from "react";
import { Avatar } from "@/components/dashboard/avatar";
import { Legend, StatusDonut, WeeklyBars, WorkerBars } from "@/components/dashboard/charts";
import { CopyButton } from "@/components/dashboard/copy-button";
import { EarningsChart } from "@/components/dashboard/earnings-chart";
import { FaucetButton } from "@/components/dashboard/faucet-button";
import { Icon } from "@/components/dashboard/icons";
import type { NavItem } from "@/components/dashboard/nav";
import { DashboardShell } from "@/components/dashboard/shell";
import { ProgressRing, WorkerMetrics } from "@/components/dashboard/worker-live";
import { BackupEmailForm } from "@/components/email-login";
import { CompanyForm, OfferActions } from "@/components/employer";
import { IntentQuestion, SwitchIntent } from "@/components/onboarding";
import { PayoutBadge, PayoutDetail, Txid } from "@/components/payouts";
import { PlainShell } from "@/components/plain-shell";
import { AutoRefresh, WorkerStreamPanel } from "@/components/worker";
import type { SessionUser } from "@/lib/auth";
import {
  DAY,
  dailySeries,
  dayLabel,
  earnedAt,
  formatZats,
  initials,
  monthBounds,
  rangeLabel,
  statusCounts,
  streamState,
  weeklyWithdrawals,
} from "@/lib/chart-data";
import { employerCompany, type Company } from "@/lib/company";
import { accountEmail } from "@/lib/email-login";
import { compactZec, formatTime } from "@/lib/format";
import { PayrollApiError, payroll, type FlowStream, type PayoutWalletSummary, type VestingView } from "@/lib/payroll";
import { requirePageUser } from "@/lib/session";
import { pct, phaseLabel } from "@/lib/vesting";

export const metadata = { title: "Home" };
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
  return user.intent === "pay" ? <EmployerHome user={user} /> : <WorkerHome user={user} />;
}

// ---- shared pieces --------------------------------------------------------------

const cardClass = "flex min-w-0 scroll-mt-4 flex-col gap-4 rounded-xl border border-linen bg-paper p-[18px] sm:px-6 sm:py-[22px]";

function Card({ id, title, aside, children, className = "" }: { id?: string; title: ReactNode; aside?: ReactNode; children: ReactNode; className?: string }) {
  const h = id ? `${id}-title` : undefined;
  return (
    <section id={id} aria-labelledby={h} className={`${cardClass} ${className}`}>
      <div className="flex items-center justify-between gap-3">
        <h2 id={h} className="m-0 text-[17px] font-semi tracking-[-0.2px]">
          {title}
        </h2>
        {aside}
      </div>
      {children}
    </section>
  );
}

function RangeChip({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex h-8 flex-none items-center gap-1.5 rounded-lg border border-linen bg-cream px-2.5 text-[13.5px] font-semi whitespace-nowrap">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#e89b01" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
        <rect x="3" y="5" width="18" height="16" rx="2" />
        <path d="M3 10h18M8 3v4M16 3v4" />
      </svg>
      {children}
    </span>
  );
}

function Kpi({
  label,
  swatch,
  value,
  unit = "ZEC",
  sub,
  dark = false,
  pill,
  className = "",
}: {
  label: string;
  swatch?: string;
  value: ReactNode;
  unit?: string;
  sub?: ReactNode;
  dark?: boolean;
  pill?: string;
  className?: string;
}) {
  return (
    <div className={`flex min-w-0 flex-col gap-2 rounded-xl border px-5 py-[18px] ${dark ? "border-espresso bg-espresso text-cream" : "border-linen bg-paper"} ${className}`}>
      <span className={`flex items-center gap-2 text-[13px] ${dark ? "text-oat" : "text-bark"}`}>
        {swatch && <i aria-hidden="true" className="inline-block size-2.5 rounded-[3px]" style={{ background: swatch }} />}
        {label}
        {pill && <span className="rounded-full bg-honey px-[7px] py-px text-[11px] font-semi text-espresso">{pill}</span>}
      </span>
      <span className={`mono truncate text-[28px] leading-[1.1] font-medium tracking-[-0.6px] ${dark ? "text-amber" : ""}`}>
        {value} <span className={`font-landing text-sm font-normal tracking-normal ${dark ? "text-oat" : "text-bark"}`}>{unit}</span>
      </span>
      {sub && <span className={`text-[12.5px] ${dark ? "text-oat" : "text-bark"}`}>{sub}</span>}
    </div>
  );
}

function EmptyBox({ children }: { children: ReactNode }) {
  return <div className="rounded-xl border-[1.5px] border-dashed border-stone px-5 py-10 text-center text-[15px] leading-normal text-bark">{children}</div>;
}

/** "Now" for this request (server components render once per request). */
function requestTime(): number {
  return Math.floor(Date.now() / 1000);
}

const sumOf = (streams: FlowStream[], f: (s: FlowStream) => number) => streams.reduce((a, s) => a + BigInt(f(s)), 0n);

function SignInEmail({ email }: { email: string | null }) {
  return (
    <Card id="sign-in-email" title="Sign-in email">
      <p className="m-0 text-sm leading-normal text-bark">
        {email ? (
          <>
            You can get a sign-in link at <b className="font-semi break-all text-espresso">{email}</b>, for example if you lose your passkey.
          </>
        ) : (
          "Add an email so you can still sign in with an email link if you lose your passkey."
        )}
      </p>
      <BackupEmailForm current={email} />
    </Card>
  );
}

// ---- vesting (both sides) -----------------------------------------------------

function VestingTag({ v }: { v: VestingView }) {
  const tone =
    v.phase === "vesting"
      ? "border border-linen"
      : v.phase === "fully_vested"
        ? "bg-espresso text-cream"
        : v.phase === "draft"
          ? "bg-honey-soft"
          : v.phase === "cancelled"
            ? "bg-mist"
            : "border border-linen bg-cream";
  return <span className={`inline-flex rounded-full px-2.5 py-[3px] text-[12.5px] font-semi whitespace-nowrap ${tone}`}>{phaseLabel(v.phase)}</span>;
}

/** Vested / paid / on its way, as parts of the total (server figures). */
function VestingBar({ v }: { v: VestingView }) {
  const b = v.balance;
  return (
    <div className="relative h-2 overflow-hidden rounded bg-mist" aria-hidden="true">
      <span className="absolute inset-y-0 left-0 bg-earned" style={{ width: `${pct(b.vested_zats, b.total_zats)}%` }} />
      <span className="absolute inset-y-0 left-0 bg-withdrawn" style={{ width: `${pct(b.paid_zats, b.total_zats)}%` }} />
      <span className="stripe-held absolute inset-y-0" style={{ left: `${pct(b.paid_zats, b.total_zats)}%`, width: `${pct(b.reserved_zats, b.total_zats)}%` }} />
    </div>
  );
}

// ---- employer ----------------------------------------------------------------

function StreamTag({ s }: { s: FlowStream }) {
  const state = streamState(s);
  if (state === "ended") return <span className="inline-flex rounded-full bg-mist px-2.5 py-[3px] text-[12.5px] font-semi whitespace-nowrap">Ended</span>;
  if (state === "withdrawing")
    return <span className="inline-flex rounded-full bg-withdrawn-soft px-2.5 py-[3px] text-[12.5px] font-semi whitespace-nowrap text-withdrawn-ink">Withdrawing</span>;
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-linen px-2.5 py-[3px] text-[12.5px] font-semi whitespace-nowrap">
      <span aria-hidden="true" className="size-[7px] rounded-full bg-earned" />
      Active
    </span>
  );
}

async function treasury(user: SessionUser, company: Company): Promise<PayoutWalletSummary | string> {
  try {
    return await payroll.payoutWallet(user.id, company.id);
  } catch (e) {
    if (e instanceof PayrollApiError && e.status < 500) return e.message;
    if (e instanceof PayrollApiError) return "The payout wallet is unavailable right now.";
    throw e;
  }
}

function CompanyStep() {
  return (
    <PlainShell>
      <div className="landing force-light font-landing text-espresso">
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
      </div>
    </PlainShell>
  );
}

async function EmployerHome({ user }: { user: SessionUser }) {
  const company = await employerCompany(user);
  if (!company) return <CompanyStep />;
  const [home, wallet, email, vesting] = await Promise.all([
    payroll.employerHome(user.id, company.id),
    treasury(user, company),
    accountEmail(user.id),
    payroll.orgVesting(user.id, company.id),
  ]);
  const now = requestTime();
  const streams = home.streams;
  const waiting = home.offers.filter((o) => o.status === "open");
  const payments = streams
    .flatMap((s) => s.payouts.map((p) => ({ p, worker: s.worker_name, streamId: s.id })))
    .sort((a, b) => b.p.created_at - a.p.created_at);

  // Metric cards: sums in zatoshis, formatted once.
  const committed = sumOf(streams, (s) => s.total_zats);
  const earned = sumOf(streams, (s) => s.balance.earned_zats);
  const claimed = sumOf(streams, (s) => s.balance.claimed_zats);
  const reserved = sumOf(streams, (s) => s.balance.reserved_zats);
  const from = now - 30 * DAY;
  const earned30 = streams.reduce((a, s) => a + earnedAt(s, now) - earnedAt(s, from), 0n);
  // What the company's payout wallet must cover: withdrawable now, and the rest of every stream.
  const owedNow = sumOf(streams, (s) => s.balance.available_zats);
  const spendable = typeof wallet === "string" ? null : BigInt(wallet.spendable_zats);
  const shortNow = spendable !== null && owedNow > spendable;

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

  const nav: NavItem[] = [
    { href: "#overview", label: "Overview", icon: "overview", current: true },
    { href: "#streams", label: "Streams", icon: "streams", count: streams.length },
    { href: "#waiting", label: "Waiting to join", icon: "waiting", count: waiting.length, alert: true },
    { href: "#vesting", label: "Vesting", icon: "vesting", count: vesting.length },
    { href: "#payments", label: "Payments", icon: "payments" },
    { href: "#wallet", label: "Payout wallet", icon: "treasury" },
  ];
  const tabs: NavItem[] = [
    { href: "#overview", label: "Overview", icon: "overview", current: true },
    { href: "#streams", label: "Streams", icon: "streams" },
    { href: "#payments", label: "Payments", icon: "payments" },
    { href: "#wallet", label: "Wallet", icon: "treasury" },
  ];

  const newStream = (
    <Link href="/app/streams/new" className="btn-amber inline-flex h-[42px] items-center justify-center gap-2 rounded-lg px-[18px] text-[15px] font-semi no-underline max-sm:h-12 max-sm:w-full">
      <Icon name="plus" size={15} strokeWidth={2.4} />
      New stream
    </Link>
  );

  return (
    <DashboardShell
      role="employer"
      displayName={user.displayName}
      company={company.name}
      nav={nav}
      account={[{ href: "#sign-in-email", label: "Sign-in email", icon: "email" }]}
      tabs={tabs}
    >
      <AutoRefresh seconds={30} />
      {/* Title row */}
      <div className="flex flex-wrap items-end justify-between gap-4 max-sm:order-1 max-sm:flex-col max-sm:items-stretch">
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-3.5 sm:hidden">
            <span aria-hidden="true" className="flex size-12 flex-none items-center justify-center rounded-[11px] bg-espresso text-base font-semi text-cream">
              {initials(company.name)}
            </span>
            <h1 className="m-0 text-[26px] leading-[1.1] font-semi tracking-[-0.6px]">{company.name}</h1>
          </div>
          <h1 className="m-0 text-[30px] font-semi tracking-[-0.6px] max-sm:hidden">Overview</h1>
          <p className="m-0 text-sm leading-[1.4] text-bark sm:text-[15px]">Streams you pay, and the treasury they&apos;re paid from.</p>
        </div>
        <div className="flex items-center gap-2.5 max-sm:flex-col max-sm:items-stretch">
          <span className="mono text-[12.5px] text-bark max-sm:hidden">As of {formatTime(now)}</span>
          {newStream}
        </div>
      </div>

      {/* Metric cards */}
      <div className="grid grid-cols-2 gap-4 max-sm:order-3 max-sm:grid-cols-1 min-[1180px]:grid-cols-4">
        <Kpi className="max-sm:hidden" label="Committed" value={formatZats(committed)} sub={`Across ${streams.length} ${streams.length === 1 ? "stream" : "streams"}`} />
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
        <Kpi
          className="max-sm:hidden"
          label="Withdrawn"
          swatch="#7a5cc8"
          value={formatZats(claimed)}
          sub={reserved > 0n ? `Plus ${formatZats(reserved)} on its way` : undefined}
        />
        <section
          id="wallet"
          aria-label="Payout wallet"
          className="flex min-w-0 scroll-mt-4 flex-col gap-2 rounded-xl border border-espresso bg-espresso px-5 py-[18px] text-cream"
        >
          <span className="flex items-center gap-2 text-[13px] text-oat">
            Payout wallet
            <span className="rounded-full bg-honey px-[7px] py-px text-[11px] font-semi text-espresso">Testnet</span>
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

      {streams.length > 0 && (
        <>
          {/* Earned vs withdrawn + streams by status */}
          <div className="grid gap-4 max-sm:order-2 min-[1180px]:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
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

          {/* Each worker's stream + withdrawals per week */}
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

      {/* Streams, then waiting (both full width) */}
      <div className="flex flex-col gap-4 max-sm:contents min-[900px]:gap-5">
        <Card
          id="streams"
          title="Streams"
          className="max-sm:order-4"
          aside={
            streams.length > 0 && (
              <span className="text-[12.5px] text-bark">
                <span className="max-sm:hidden">Updates every 30 seconds</span>
                <span className="sm:hidden">
                  {streams.length} {streams.length === 1 ? "stream" : "streams"} · {formatZats(committed)} ZEC
                </span>
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

        {waiting.length > 0 && (
          <Card id="waiting" title="Waiting for the worker to join" className="max-sm:order-5">
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
          </Card>
        )}
      </div>

      {/* Vesting agreements (after streams: vesting is the additional option) */}
      <Card
        id="vesting"
        title="Vesting"
        className="max-sm:order-5"
        aside={
          <Link href="/app/vesting/new" className="text-sm font-semi whitespace-nowrap text-espresso underline decoration-amber-deep decoration-2 underline-offset-4">
            New agreement →
          </Link>
        }
      >
        {vesting.length === 0 ? (
          <EmptyBox>No vesting agreements. Grant someone in your company a ZEC allocation that vests over time, with an optional cliff.</EmptyBox>
        ) : (
          <ul className="m-0 flex list-none flex-col p-0">
            {vesting.map((v) => (
              <li key={v.id} className="flex flex-col gap-2 border-b border-mist py-3.5 first:pt-0 last:border-0 last:pb-0">
                <div className="flex items-center justify-between gap-2.5">
                  <Link href={`/app/vesting/${v.id}`} className="flex min-w-0 items-center gap-2.5 font-semi text-espresso no-underline hover:underline">
                    <Avatar name={v.employee_display_name} />
                    <span className="truncate">{v.employee_display_name}</span>
                  </Link>
                  <VestingTag v={v} />
                </div>
                <VestingBar v={v} />
                <div className="flex flex-wrap justify-between gap-x-4 gap-y-1 text-[12.5px] text-bark">
                  <span>
                    Vested <span className="mono text-espresso">{compactZec(v.balance.vested_zec)}</span> of <span className="mono">{compactZec(v.total_zec)}</span> ZEC
                  </span>
                  <span>
                    Paid <span className="mono text-espresso">{compactZec(v.balance.paid_zec)}</span>
                    {v.balance.reserved_zats > 0 && (
                      <>
                        {" "}
                        · on its way <span className="mono">{compactZec(v.balance.reserved_zec)}</span>
                      </>
                    )}
                  </span>
                  <span>
                    {v.status === "draft"
                      ? v.activatable
                        ? `Activate by ${formatTime(v.start_at)}`
                        : "Start passed: discard and recreate"
                      : `Ends ${formatTime(v.end_at)}`}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {/* Payments (full width) */}
      <Card id="payments" title="Payments" className="max-sm:order-6">
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

      <div className="max-sm:order-7">
        <SignInEmail email={email} />
      </div>
      <p className="m-0 text-center max-sm:order-8 sm:hidden">
        <SwitchIntent to="get_paid" className="min-h-11 cursor-pointer border-0 bg-transparent p-0 text-sm text-bark underline underline-offset-[3px] disabled:opacity-60">
          Get paid through hushush →
        </SwitchIntent>
      </p>
    </DashboardShell>
  );
}

// ---- worker ------------------------------------------------------------------

function WorkerStreamCard({ s }: { s: FlowStream }) {
  const address = s.address_locked ? (
    <span className="inline-flex items-center gap-1.5 self-start rounded-full border border-linen bg-cream px-[9px] py-[3px] text-[12.5px] whitespace-nowrap text-bark">
      <Icon name="lock" size={11} strokeWidth={2.4} />
      Payout address locked
    </span>
  ) : (
    <div className="flex items-center justify-between gap-2.5 rounded-lg border border-honey bg-honey-soft px-3.5 py-3">
      <span className="text-[13.5px] leading-[1.4]">Set your payout address to withdraw.</span>
      <Link href={`/app/streams/${s.id}`} className="text-[13.5px] font-semi whitespace-nowrap text-espresso underline decoration-amber-deep decoration-2 underline-offset-4">
        Set it →
      </Link>
    </div>
  );
  return (
    <section aria-labelledby={`stream-${s.id}`} className={cardClass}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <span aria-hidden="true" className="flex size-[38px] flex-none items-center justify-center rounded-[9px] bg-espresso text-[13px] font-semi text-cream">
            {initials(s.organization_name)}
          </span>
          <div className="min-w-0">
            <h2 id={`stream-${s.id}`} className="m-0 text-[17px] font-semi tracking-[-0.2px]">
              {s.organization_name} · <span className="mono font-medium">{compactZec(s.total_zec)}</span> ZEC
            </h2>
            <div className="mt-[3px] text-[12.5px] text-bark">
              {rangeLabel(s.start_at, s.ended_at ?? s.end_at)}
              {s.status === "ended" && " (ended)"}
            </div>
          </div>
        </div>
        <Link href={`/app/streams/${s.id}`} className="text-sm font-semi whitespace-nowrap text-espresso underline decoration-amber-deep decoration-2 underline-offset-4">
          Details →
        </Link>
      </div>
      <WorkerStreamPanel s={s} address={address} quietAddressHint />
    </section>
  );
}

async function WorkerHome({ user }: { user: SessionUser }) {
  const [streams, email, vesting] = await Promise.all([payroll.workerStreams(user.id), accountEmail(user.id), payroll.myVesting(user.id)]);
  const now = requestTime();
  const missing = streams.filter((s) => !s.address_locked).length;

  const nav: NavItem[] = [
    { href: "#overview", label: "Overview", icon: "overview", current: true },
    { href: "#streams", label: "My streams", icon: "streams", count: streams.length },
    { href: "#withdrawals", label: "Withdrawals", icon: "payments" },
    { href: "#vesting", label: "Vesting", icon: "vesting", count: vesting.length },
    { href: "#streams", label: "Payout addresses", icon: "lock", count: missing, alert: true },
  ];
  const tabs: NavItem[] = [
    { href: "#overview", label: "Overview", icon: "overview", current: true },
    { href: "#streams", label: "Streams", icon: "streams" },
    { href: "#withdrawals", label: "Withdrawals", icon: "payments" },
    { href: "#streams", label: "Addresses", icon: "lock", count: missing, alert: true },
  ];

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

  const shell = (children: ReactNode) => (
    <DashboardShell role="worker" displayName={user.displayName} nav={nav} account={[{ href: "#sign-in-email", label: "Sign-in email", icon: "email" }]} tabs={tabs}>
      <AutoRefresh seconds={15} />
      {header}
      {children}
      <SignInEmail email={email} />
    </DashboardShell>
  );

  const vestingSection =
    vesting.length > 0 ? (
      <Card id="vesting" title="Vesting">
        <ul className="m-0 flex list-none flex-col p-0">
          {vesting.map((v) => (
            <li key={v.id} className="flex flex-col gap-2 border-b border-mist py-3.5 first:pt-0 last:border-0 last:pb-0">
              <div className="flex items-center justify-between gap-2.5">
                <span className="min-w-0 truncate font-semi">
                  {v.organization_name} · <span className="mono font-medium">{compactZec(v.total_zec)}</span> ZEC
                </span>
                <VestingTag v={v} />
              </div>
              <VestingBar v={v} />
              <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 text-[12.5px] text-bark">
                <span>
                  Vested <span className="mono text-espresso">{compactZec(v.balance.vested_zec)}</span> · paid{" "}
                  <span className="mono text-espresso">{compactZec(v.balance.paid_zec)}</span>
                  {v.balance.reserved_zats > 0 && (
                    <>
                      {" "}
                      · on its way <span className="mono">{compactZec(v.balance.reserved_zec)}</span>
                    </>
                  )}
                </span>
                <span>
                  Available <span className="mono font-semi text-espresso">{compactZec(v.balance.available_zec)}</span> ZEC
                </span>
              </div>
              <div className="flex flex-wrap items-center justify-between gap-2 text-[12.5px] text-bark">
                <span>{rangeLabel(v.start_at, v.end_at)}</span>
                <Link href={`/app/vesting/${v.id}`} className="text-sm font-semi whitespace-nowrap text-espresso underline decoration-amber-deep decoration-2 underline-offset-4">
                  {v.balance.available_zats > 0 ? "Withdraw →" : "Details →"}
                </Link>
              </div>
            </li>
          ))}
        </ul>
      </Card>
    ) : null;

  if (streams.length === 0) {
    return shell(
      vestingSection ?? (
        <EmptyBox>Nobody is paying you through hushush yet. When someone does, they&apos;ll send you a link. Open it to start receiving pay.</EmptyBox>
      ),
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
  const payouts = streams.flatMap((s) => s.payouts.map((p) => ({ p, org: s.organization_name }))).sort((a, b) => b.p.created_at - a.p.created_at);
  const lastConfirmed = payouts.find((x) => x.p.state === "confirmed" && x.p.resolved_at != null);
  const lastPaid = lastConfirmed ? `${dayLabel(lastConfirmed.p.resolved_at!)} ${new Date(lastConfirmed.p.resolved_at! * 1000).getUTCFullYear()}` : null;
  const asOfKey = Math.max(...streams.map((s) => s.balance.as_of));

  return shell(
    <>
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

      <div id="streams" className="grid scroll-mt-4 items-start gap-4 min-[1180px]:grid-cols-2">
        {streams.map((s) => (
          <WorkerStreamCard key={s.id} s={s} />
        ))}
      </div>

      <Card id="withdrawals" title="Withdrawals">
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
      {vestingSection}
    </>,
  );
}
