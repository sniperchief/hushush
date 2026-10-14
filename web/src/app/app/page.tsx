import Link from "next/link";
import { CompanyForm, OfferActions } from "@/components/employer";
import { PayoutBadge, Txid } from "@/components/payouts";
import { Alert, Badge, Card, CardTitle, Empty } from "@/components/ui";
import { AutoRefresh, WorkerStreamPanel } from "@/components/worker";
import type { SessionUser } from "@/lib/auth";
import { employerCompany, type Company } from "@/lib/company";
import { compactZec, formatTime } from "@/lib/format";
import { PayrollApiError, payroll, type FlowStream, type PayoutWalletSummary } from "@/lib/payroll";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "Home" };
export const dynamic = "force-dynamic";

export default async function Home() {
  const user = await requirePageUser("/app");
  return user.kind === "employer" ? <EmployerHome user={user} /> : <WorkerHome user={user} />;
}

// ---- employer ----------------------------------------------------------------

function StreamStatus({ s }: { s: FlowStream }) {
  if (s.status === "ended") return <Badge>Ended</Badge>;
  if (s.payouts.some((p) => p.state !== "confirmed" && p.state !== "failed")) return <Badge tone="accent">Withdrawing</Badge>;
  return <Badge tone="green">● Active</Badge>;
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

async function EmployerHome({ user }: { user: SessionUser }) {
  const company = await employerCompany(user);
  if (!company) {
    return (
      <div className="mx-auto max-w-md space-y-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Name your company</h1>
          <p className="mt-1 text-sm text-ink-soft">Step 2 of 2. One employer account runs one company.</p>
        </div>
        <Card>
          <CompanyForm />
        </Card>
      </div>
    );
  }
  const [home, wallet] = await Promise.all([payroll.employerHome(user.id, company.id), treasury(user, company)]);
  const waiting = home.offers.filter((o) => o.status === "open");
  const payments = home.streams
    .flatMap((s) => s.payouts.map((p) => ({ p, worker: s.worker_name, streamId: s.id })))
    .sort((a, b) => b.p.created_at - a.p.created_at);

  return (
    <div className="space-y-6">
      <AutoRefresh seconds={30} />
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">{company.name}</h1>
          <p className="mt-1 text-sm text-ink-soft">Streams you pay, and the treasury they&apos;re paid from.</p>
        </div>
        <Link href="/app/streams/new" className="inline-flex min-h-10 items-center rounded-xl bg-ink px-4 text-sm font-medium text-surface hover:opacity-90">
          New stream
        </Link>
      </div>

      <Card>
        <CardTitle action={<Badge tone="amber">Shared testnet demo treasury</Badge>}>Treasury</CardTitle>
        {typeof wallet === "string" ? (
          <Alert tone="neutral">{wallet}</Alert>
        ) : (
          <div className="grid gap-4 sm:grid-cols-[auto_1fr] sm:items-start">
            <div>
              <div className="text-xs font-medium uppercase tracking-wide text-ink-soft">Spendable</div>
              <div className="mt-1 font-mono text-2xl font-semibold tabular-nums">
                {compactZec(wallet.spendable_zec)} <span className="text-sm font-normal text-ink-soft">TAZ</span>
              </div>
              {wallet.pending_zats > 0 && <div className="text-xs text-ink-soft">+{compactZec(wallet.pending_zec)} TAZ settling</div>}
            </div>
            <div className="min-w-0">
              <div className="text-xs font-medium uppercase tracking-wide text-ink-soft">Funding address (testnet)</div>
              <div className="mt-1 break-all rounded-xl bg-muted px-3 py-2 font-mono text-xs">{wallet.funding_address}</div>
            </div>
            <p className="text-xs text-ink-soft sm:col-span-2">
              This demo runs one shared testnet payout wallet for every company on it, so the balance is not yours alone. Withdrawals and their
              network fees are paid from it. Testnet only — TAZ has no value.
            </p>
          </div>
        )}
      </Card>

      <Card>
        <CardTitle>Streams</CardTitle>
        {home.streams.length === 0 ? (
          <Empty>No worker has joined a stream yet. Create one and send them the worker link.</Empty>
        ) : (
          <div className="-mx-5 overflow-x-auto sm:mx-0">
            <table className="w-full min-w-[640px] text-sm">
              <thead>
                <tr className="border-b border-line text-left text-xs uppercase tracking-wide text-ink-soft">
                  <th className="px-5 py-2 font-medium sm:px-2">Worker</th>
                  <th className="px-2 py-2 text-right font-medium">Total</th>
                  <th className="px-2 py-2 text-right font-medium">Earned</th>
                  <th className="px-2 py-2 text-right font-medium">Withdrawn</th>
                  <th className="px-2 py-2 font-medium">Status</th>
                  <th className="px-2 py-2 font-medium">Ends</th>
                </tr>
              </thead>
              <tbody>
                {home.streams.map((s) => (
                  <tr key={s.id} className="border-b border-line last:border-0">
                    <td className="px-5 py-3 sm:px-2">
                      <Link href={`/app/streams/${s.id}`} className="font-medium hover:underline">
                        {s.worker_name}
                      </Link>
                    </td>
                    <td className="px-2 py-3 text-right font-mono tabular-nums">{compactZec(s.total_zec)}</td>
                    <td className="px-2 py-3 text-right font-mono tabular-nums">{compactZec(s.balance.earned_zec)}</td>
                    <td className="px-2 py-3 text-right font-mono tabular-nums">{compactZec(s.balance.claimed_zec)}</td>
                    <td className="px-2 py-3">
                      <StreamStatus s={s} />
                    </td>
                    <td className="px-2 py-3 text-ink-soft">{formatTime(s.ended_at ?? s.end_at)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      {waiting.length > 0 && (
        <Card>
          <CardTitle>Waiting for the worker to join</CardTitle>
          <ul className="divide-y divide-line">
            {waiting.map((o) => (
              <li key={o.id} className="space-y-3 py-4 first:pt-0 last:pb-0">
                <div className="flex flex-wrap items-baseline justify-between gap-2">
                  <span className="font-medium">{o.worker_name}</span>
                  <span className="font-mono text-sm tabular-nums">{compactZec(o.total_zec)} ZEC</span>
                </div>
                <p className="text-xs text-ink-soft">
                  {formatTime(o.start_at)} → {formatTime(o.end_at)}. Earnings count from the start date even if they join later.
                </p>
                <OfferActions offerId={o.id} />
              </li>
            ))}
          </ul>
        </Card>
      )}

      <Card>
        <CardTitle>Payments</CardTitle>
        {payments.length === 0 ? (
          <Empty>No withdrawals yet.</Empty>
        ) : (
          <ul className="divide-y divide-line">
            {payments.map(({ p, worker, streamId }) => (
              <li key={p.id} className="flex flex-wrap items-center justify-between gap-2 py-3 text-sm">
                <div className="min-w-0">
                  <Link href={`/app/streams/${streamId}`} className="font-medium hover:underline">
                    {worker}
                  </Link>{" "}
                  <span className="font-mono tabular-nums">{compactZec(p.amount_zec)} ZEC</span>
                  <div className="text-xs text-ink-soft">
                    {formatTime(p.created_at)}
                    {p.txid && (
                      <>
                        {" · "}
                        <Txid txid={p.txid} />
                      </>
                    )}
                  </div>
                </div>
                <PayoutBadge p={p} />
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  );
}

// ---- worker ------------------------------------------------------------------

async function WorkerHome({ user }: { user: SessionUser }) {
  const streams = await payroll.workerStreams(user.id);
  return (
    <div className="space-y-6">
      <AutoRefresh seconds={15} />
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Hi {user.displayName}</h1>
        <p className="mt-1 text-sm text-ink-soft">What you&apos;re earning, live.</p>
      </div>
      {streams.length === 0 ? (
        <Empty>No streams yet. Open the worker link your employer sent you.</Empty>
      ) : (
        streams.map((s) => (
          <Card key={s.id}>
            <CardTitle
              action={
                <Link href={`/app/streams/${s.id}`} className="text-sm text-ink-soft hover:underline">
                  Details →
                </Link>
              }
            >
              {s.organization_name} · {compactZec(s.total_zec)} ZEC
            </CardTitle>
            <p className="-mt-2 mb-4 text-xs text-ink-soft">
              {formatTime(s.start_at)} → {formatTime(s.ended_at ?? s.end_at)}
              {s.status === "ended" && " (ended)"}
              {" · "}
              {s.address_locked ? "🔒 payout address locked" : "payout address not set yet"}
            </p>
            <WorkerStreamPanel s={s} />
          </Card>
        ))
      )}
    </div>
  );
}
