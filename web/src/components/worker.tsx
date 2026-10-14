"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api } from "@/lib/client-api";
import type { FlowStream } from "@/lib/payroll";
import { Alert, Button, Field, cx, inputClass } from "./ui";

/** Re-renders server data periodically so server-computed state stays fresh. */
export function AutoRefresh({ seconds = 15 }: { seconds?: number }) {
  const router = useRouter();
  useEffect(() => {
    const t = setInterval(() => router.refresh(), seconds * 1000);
    return () => clearInterval(t);
  }, [router, seconds]);
  return null;
}

const COIN = BigInt(100_000_000);

/** Zatoshis → "1.23456789" (exact; BigInt). */
function zec(zats: bigint): string {
  const neg = zats < 0;
  const a = neg ? -zats : zats;
  return `${neg ? "-" : ""}${a / COIN}.${(a % COIN).toString().padStart(8, "0")}`;
}

/**
 * Live figures: the server's accrual formula re-evaluated every second for
 * display, earned(t) = floor(total × (min(t, end) − start) / (end − start)),
 * anchored at the server's `as_of` (the parent re-keys this component on every
 * refresh). The server stays authoritative: a withdrawal pays what it computes.
 */
function LiveFigures({ s }: { s: FlowStream }) {
  const [anchor] = useState(() => Date.now());
  const [now, setNow] = useState(anchor);
  const active = s.status === "active";
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active]);

  const total = BigInt(s.total_zats);
  const serverEarned = BigInt(s.balance.earned_zats);
  let earned = serverEarned;
  if (active) {
    const t = s.balance.as_of + Math.floor((now - anchor) / 1000);
    const elapsed = BigInt(Math.max(0, Math.min(t, s.end_at) - s.start_at));
    const e = (total * elapsed) / BigInt(s.end_at - s.start_at);
    if (e > earned) earned = e;
  }
  const withdrawn = BigInt(s.balance.claimed_zats);
  const held = BigInt(s.balance.reserved_zats);
  const available = earned - withdrawn - held;

  return (
    <>
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="rounded-xl border border-line bg-accent-bg p-4 sm:col-span-2">
          <div className="text-xs font-medium uppercase tracking-wide text-ink-soft">Available to withdraw</div>
          <div className="mt-1 font-mono text-2xl font-semibold tabular-nums sm:text-3xl">
            {zec(available > BigInt(0) ? available : BigInt(0))} <span className="text-base font-normal text-ink-soft">ZEC</span>
          </div>
          <div className="mt-1 text-xs text-ink-soft">
            Earned {zec(earned)} of {zec(total)} ZEC{active ? " · ticking every second" : " · stream ended"}
          </div>
        </div>
        <div className="rounded-xl border border-line p-4">
          <div className="text-xs font-medium uppercase tracking-wide text-ink-soft">Withdrawn</div>
          <div className="mt-1 font-mono text-lg font-semibold tabular-nums">
            {zec(withdrawn)} <span className="text-sm font-normal text-ink-soft">ZEC</span>
          </div>
          {held > BigInt(0) && <div className="mt-1 text-xs text-ink-soft">{zec(held)} ZEC on its way</div>}
        </div>
      </div>
      <Progress total={total} earned={earned} withdrawn={withdrawn} held={held} />
    </>
  );
}

function Progress({ total, earned, withdrawn, held }: { total: bigint; earned: bigint; withdrawn: bigint; held: bigint }) {
  const pct = (v: bigint) => (total > BigInt(0) ? Number((v * BigInt(10_000)) / total) / 100 : 0);
  return (
    <div className="space-y-1.5">
      <div
        className="relative h-3 overflow-hidden rounded-full bg-muted"
        role="progressbar"
        aria-label="Earned so far"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.floor(pct(earned))}
      >
        <div className="absolute inset-y-0 left-0 bg-accent/40" style={{ width: `${pct(earned)}%` }} />
        <div className="absolute inset-y-0 left-0 bg-ok" style={{ width: `${pct(withdrawn)}%` }} />
        <div className="absolute inset-y-0 bg-accent" style={{ left: `${pct(withdrawn)}%`, width: `${pct(held)}%` }} />
        {[20, 40, 60, 80].map((m) => (
          <div key={m} className="absolute inset-y-0 w-px bg-surface" style={{ left: `${m}%` }} aria-hidden />
        ))}
      </div>
      <div className="flex justify-between text-[11px] text-ink-faint" aria-hidden>
        {[0, 20, 40, 60, 80, 100].map((m) => (
          <span key={m}>{m}%</span>
        ))}
      </div>
    </div>
  );
}

/** Live balance, progress with 20% milestones, and the Withdraw action. */
export function WorkerStreamPanel({ s }: { s: FlowStream }) {
  const router = useRouter();
  const open = s.payouts.find((p) => p.state === "reserved" || p.state === "submitted" || p.state === "requires_reconciliation");
  // The server decides; pages refresh often enough to pick up a new milestone.
  const unlocked = s.withdrawable_zats !== null;

  // One key per withdrawal attempt: a double tap or a retry reuses it.
  const [key, setKey] = useState(() => crypto.randomUUID());
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

  let hint: string;
  if (!s.payouts_enabled) hint = "Withdrawals aren't enabled on this server.";
  else if (!s.address_locked) hint = "Set your payout address to withdraw.";
  else if (open) hint = "A withdrawal is in progress.";
  else if (unlocked) hint = `Pays ${s.withdrawable_zec} ZEC — everything available — to your locked address.`;
  else if (s.status === "ended") hint = "Everything has been withdrawn.";
  else hint = `Unlocks when ${zec(BigInt(s.milestone_zats))} ZEC (20% of the total) is available.`;

  return (
    <div className="space-y-5">
      <LiveFigures key={s.balance.as_of} s={s} />
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <p className="text-sm text-ink-soft">{hint}</p>
        <Button
          disabled={!unlocked || pending || Boolean(open)}
          className="sm:min-w-40"
          onClick={async () => {
            setPending(true);
            setError(undefined);
            const r = await api(`/api/streams/${s.id}/withdraw`, "POST", { idempotencyKey: key });
            setPending(false);
            if (!r.ok) setError(r.error);
            else setKey(crypto.randomUUID());
            router.refresh();
          }}
        >
          {pending ? "Sending… (≈1 min)" : "Withdraw"}
        </Button>
      </div>
      {pending && (
        <Alert tone="neutral">Building your shielded transaction. This takes about a minute — it will never be sent twice, even if you tap again or reload.</Alert>
      )}
      {error && <Alert>{error}</Alert>}
    </div>
  );
}

/** Sets the stream's payout address — once; it is then locked. */
export function AddressForm({ streamId }: { streamId: string }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const [confirming, setConfirming] = useState<string>();

  return (
    <form
      className="space-y-3"
      onSubmit={async (e) => {
        e.preventDefault();
        const address = String(new FormData(e.currentTarget).get("address") ?? "").trim();
        if (confirming !== address) {
          setConfirming(address);
          return;
        }
        setPending(true);
        setError(undefined);
        const r = await api(`/api/streams/${streamId}/address`, "POST", { address });
        setPending(false);
        if (!r.ok) {
          setConfirming(undefined);
          return setError(r.error);
        }
        router.refresh();
      }}
    >
      <Field label="Payout address" hint="A Zcash testnet Unified Address with an Orchard receiver (for example from Zashi in testnet mode).">
        <input
          name="address"
          required
          autoComplete="off"
          spellCheck={false}
          placeholder="utest1…"
          onChange={() => setConfirming(undefined)}
          className={cx(inputClass, "font-mono")}
        />
      </Field>
      {confirming && (
        <Alert tone="amber">
          This address will be <strong>locked for the life of this stream</strong> — it can never be changed. Check it, then press the button again.
        </Alert>
      )}
      {error && <Alert>{error}</Alert>}
      <Button type="submit" disabled={pending}>
        {pending ? "Checking…" : confirming ? "Lock this address" : "Set payout address"}
      </Button>
    </form>
  );
}
