"use client";

import { useRouter } from "next/navigation";
import { useState, useSyncExternalStore } from "react";
import { api } from "@/lib/client-api";
import { compactZec } from "@/lib/format";
import type { QuoteView, VestingDetail, VestingView } from "@/lib/payroll";
import { CANCELLATION_RULE, DAY, cliffShare, parseZats, scheduleSentences, zecOf } from "@/lib/vesting";
import { PayoutDetail } from "./payouts";
import { Alert, Button, Field, cx, inputClass } from "./ui";

export interface EligibleEmployee {
  id: string;
  name: string;
}

function localInput(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

const noSubscribe = () => () => {};
/** Tomorrow at 09:00 in the browser's time zone. */
const defaultStartInput = () => {
  const d = new Date(Date.now() + 86_400_000);
  d.setHours(9, 0, 0, 0);
  return localInput(d);
};

interface Draft {
  employee: EligibleEmployee;
  totalZec: string;
  total: bigint;
  startAt: number;
  durationDays: number;
  cliffDays: number;
}

/** Create a vesting agreement: enter the terms, review them, create a draft. */
export function NewVestingForm({ employees }: { employees: EligibleEmployee[] }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const [review, setReview] = useState<Draft>();
  // In the browser's time zone, so only known client-side ("" on the server).
  const defaultStart = useSyncExternalStore(noSubscribe, defaultStartInput, () => "");

  if (employees.length === 0) {
    return (
      <Alert tone="neutral">
        Vesting agreements are for people already in your company. Someone joins your company by accepting a stream&apos;s worker link; once they
        have, you can grant them vesting here.
      </Alert>
    );
  }

  if (review) {
    const durationSecs = review.durationDays * DAY;
    const cliffSecs = review.cliffDays * DAY;
    const lines = scheduleSentences({
      total_zec: zecOf(review.total),
      start_at: review.startAt,
      end_at: review.startAt + durationSecs,
      duration_secs: durationSecs,
      cliff_secs: cliffSecs,
      cliff_at: review.startAt + cliffSecs,
      cliff_vests_zec: zecOf(cliffShare(review.total, cliffSecs, durationSecs)),
    });
    return (
      <div className="space-y-4">
        <h2 className="m-0 text-[17px] font-semi tracking-[-0.2px]">Review the agreement</h2>
        <dl className="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-[max-content_1fr]">
          <dt className="text-bark">Employee</dt>
          <dd className="font-medium">{review.employee.name}</dd>
          <dt className="text-bark">Total</dt>
          <dd className="mono">{zecOf(review.total)} ZEC</dd>
          <dt className="text-bark">Method</dt>
          <dd>Linear{review.cliffDays > 0 ? `, with a ${review.cliffDays}-day cliff` : ", no cliff"}</dd>
        </dl>
        <div className="space-y-2 rounded-lg border border-linen bg-cream px-4 py-3 text-sm leading-relaxed">
          {lines.map((l) => (
            <p key={l} className="m-0">
              {l}
            </p>
          ))}
          <p className="m-0 text-bark">{CANCELLATION_RULE}</p>
        </div>
        <Alert tone="amber">
          This creates a <strong>draft</strong>. Nothing vests until you activate it, and it must be activated before its start date. The employee
          can&apos;t see it until then.
        </Alert>
        {error && <Alert>{error}</Alert>}
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={pending}
            onClick={async () => {
              setPending(true);
              setError(undefined);
              const r = await api<{ agreement: VestingView }>("/api/vesting", "POST", {
                employeeUserId: review.employee.id,
                totalZec: review.totalZec,
                startAt: review.startAt,
                durationDays: review.durationDays,
                cliffDays: review.cliffDays > 0 ? review.cliffDays : null,
              });
              if (!r.ok || !r.data) {
                setPending(false);
                return setError(r.error);
              }
              router.push(`/app/vesting/${r.data.agreement.id}`);
            }}
          >
            {pending ? "Creating…" : "Create draft"}
          </Button>
          <Button variant="secondary" disabled={pending} onClick={() => setReview(undefined)}>
            Back
          </Button>
        </div>
      </div>
    );
  }

  return (
    <form
      className="grid gap-4 sm:grid-cols-2"
      onSubmit={(e) => {
        e.preventDefault();
        setError(undefined);
        const fd = new FormData(e.currentTarget);
        const employee = employees.find((m) => m.id === fd.get("employee"));
        const totalZec = String(fd.get("totalZec") ?? "").trim();
        const total = parseZats(totalZec);
        const startMs = new Date(String(fd.get("startAt") ?? "")).getTime();
        const durationDays = Number(fd.get("durationDays"));
        const cliffDays = Number(fd.get("cliffDays") || 0);
        if (!employee) return setError("Choose an employee.");
        if (total === null) return setError("Enter a positive ZEC amount with at most 8 decimal places.");
        if (!Number.isFinite(startMs) || startMs <= Date.now()) return setError("Choose a start in the future.");
        if (!Number.isInteger(durationDays) || durationDays < 1 || durationDays > 36_500) return setError("The duration must be 1 to 36,500 days.");
        if (!Number.isInteger(cliffDays) || cliffDays < 0 || cliffDays > durationDays) return setError("The cliff must be between 0 days and the duration.");
        setReview({ employee, totalZec, total, startAt: Math.floor(startMs / 1000), durationDays, cliffDays });
      }}
    >
      <Field label="Employee" hint="People who have joined your company.">
        <select name="employee" required className={inputClass} defaultValue="">
          <option value="" disabled>
            Choose…
          </option>
          {employees.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Total (ZEC)" hint="The whole allocation. It vests over the duration.">
        <input name="totalZec" required inputMode="decimal" placeholder="10" pattern="[0-9]+(\.[0-9]{1,8})?" className={`${inputClass} mono`} />
      </Field>
      <Field label="Start" hint="Vesting is counted from here. You activate the agreement before this.">
        <input type="datetime-local" name="startAt" required defaultValue={defaultStart} key={defaultStart} className={inputClass} />
      </Field>
      <Field label="Duration (days)" hint="Everything has vested at the end.">
        <input type="number" name="durationDays" required min={1} max={36_500} step={1} defaultValue={365} className={inputClass} />
      </Field>
      <Field
        label="Cliff (days, optional)"
        hint="Nothing vests before the cliff. At the cliff, the share for those days vests at once; then vesting continues evenly."
      >
        <input type="number" name="cliffDays" min={0} max={36_500} step={1} defaultValue={0} className={inputClass} />
      </Field>
      {error && (
        <div className="sm:col-span-2">
          <Alert>{error}</Alert>
        </div>
      )}
      <div className="sm:col-span-2">
        <Button type="submit">Review</Button>
      </div>
    </form>
  );
}

/** Employer actions on one agreement: activate a draft, cancel. */
export function VestingActions({ v }: { v: VestingView }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const run = async (what: "activate" | "cancel", question: string) => {
    if (!confirm(question)) return;
    setPending(true);
    setError(undefined);
    const r = await api(`/api/vesting/${v.id}/${what}`);
    setPending(false);
    if (!r.ok) return setError(r.error);
    router.refresh();
  };
  const draft = v.status === "draft";
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap gap-2">
        {v.activatable && (
          <Button
            disabled={pending}
            onClick={() => run("activate", `Activate this agreement? ${v.employee_display_name} will see it, and it vests by its schedule. Its terms can't change.`)}
          >
            Activate
          </Button>
        )}
        {v.cancellable && (
          <Button
            variant="danger"
            disabled={pending}
            onClick={() =>
              run(
                "cancel",
                draft
                  ? "Discard this draft? Nothing is owed on it."
                  : `Cancel this agreement? Vesting stops now. What has vested stays owed to ${v.employee_display_name} and can still be withdrawn; the rest won't vest.`,
              )
            }
          >
            {draft ? "Discard draft" : "Cancel agreement"}
          </Button>
        )}
      </div>
      {draft && !v.activatable && (
        <p className="text-xs text-bark">The start date has passed, so this draft can no longer be activated. Discard it and create a new one.</p>
      )}
      {error && <Alert>{error}</Alert>}
    </div>
  );
}

/** Sets the agreement's payout address — once; it is then locked. */
export function VestingAddressForm({ agreementId }: { agreementId: string }) {
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
        const r = await api(`/api/vesting/${agreementId}/address`, "POST", { address });
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
          className={cx(inputClass, "mono")}
        />
      </Field>
      {confirming && (
        <Alert tone="amber">
          This address will be <strong>locked for the life of this agreement</strong> — every withdrawal goes here and it can never be changed. Check
          it, then press the button again.
        </Alert>
      )}
      {error && <Alert>{error}</Alert>}
      <Button type="submit" disabled={pending}>
        {pending ? "Checking…" : confirming ? "Lock this address" : "Set payout address"}
      </Button>
    </form>
  );
}

/**
 * The employee's withdrawal: amount → fee quote → confirm. The server checks
 * the amount against the authoritative balance at every step; the browser
 * only formats what it is told.
 */
export function VestingWithdrawPanel({ d }: { d: VestingDetail }) {
  const router = useRouter();
  const v = d.agreement;
  const open = d.payouts.find((p) => p.state === "reserved" || p.state === "submitted" || p.state === "requires_reconciliation");
  const [amount, setAmount] = useState(d.withdrawable_zec ? compactZec(d.withdrawable_zec) : "");
  const [quote, setQuote] = useState<QuoteView>();
  // One key per confirmed withdrawal: a double tap or a retry reuses it.
  const [key, setKey] = useState(() => crypto.randomUUID());
  const [pending, setPending] = useState<"quote" | "send">();
  const [error, setError] = useState<string>();

  if (!d.payouts_enabled) return <p className="text-sm text-bark">Withdrawals aren&apos;t enabled on this server.</p>;
  if (!d.address_locked) return <VestingAddressForm agreementId={v.id} />;
  if (open)
    return (
      <div role="status" className="rounded-lg border border-linen bg-cream px-4 py-3 text-sm leading-relaxed">
        <strong>A withdrawal of {compactZec(open.amount_zec)} ZEC is in progress.</strong> <PayoutDetail p={open} />
      </div>
    );
  if (d.withdrawable_zats === null) {
    let why = "Nothing is available to withdraw right now.";
    if (v.phase === "not_started") why = "Vesting hasn't started yet.";
    else if (v.phase === "in_cliff") why = "Nothing has vested yet: the cliff hasn't been reached.";
    else if (v.balance.paid_zats === v.balance.vested_zats && v.balance.vested_zats > 0) why = "Everything vested so far has been withdrawn.";
    return <p className="text-sm text-bark">{why}</p>;
  }

  if (quote) {
    const canSend = quote.valid_recipient && quote.sufficient;
    return (
      <div className="space-y-3">
        <dl className="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-[max-content_1fr]">
          <dt className="text-bark">You receive</dt>
          <dd className="mono font-semi">{compactZec(quote.amount_zec)} ZEC</dd>
          <dt className="text-bark">Network fee</dt>
          <dd>
            {quote.fee_zec ? <span className="mono">{compactZec(quote.fee_zec)} ZEC</span> : "—"}{" "}
            <span className="text-bark">(paid by {v.organization_name}&apos;s payout wallet, not from your amount)</span>
          </dd>
          <dt className="text-bark">To</dt>
          <dd className="break-all mono text-xs">{d.address ? `${d.address.slice(0, 16)}…${d.address.slice(-8)}` : "your locked address"}</dd>
          <dt className="text-bark">Paid when</dt>
          <dd>{quote.required_confirmations} confirmations on the Zcash testnet</dd>
        </dl>
        {!canSend && <Alert tone="amber">{quote.reason ?? "This withdrawal can't be sent right now."}</Alert>}
        {error && <Alert>{error}</Alert>}
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={!canSend || pending !== undefined}
            onClick={async () => {
              setPending("send");
              setError(undefined);
              const r = await api(`/api/vesting/${v.id}/withdraw`, "POST", { amountZec: quote.amount_zec, idempotencyKey: key });
              setPending(undefined);
              if (!r.ok) return setError(r.error);
              setKey(crypto.randomUUID());
              setQuote(undefined);
              router.refresh();
            }}
          >
            {pending === "send" ? "Sending… (≈1 min)" : "Confirm withdrawal"}
          </Button>
          <Button variant="secondary" disabled={pending !== undefined} onClick={() => setQuote(undefined)}>
            Change amount
          </Button>
        </div>
        {pending === "send" && (
          <p role="status" className="text-xs text-bark">
            Building your shielded transaction. This takes about a minute — it will never be sent twice, even if you tap again or reload.
          </p>
        )}
      </div>
    );
  }

  return (
    <form
      className="space-y-3"
      onSubmit={async (e) => {
        e.preventDefault();
        setError(undefined);
        if (parseZats(amount) === null) return setError("Enter a positive ZEC amount with at most 8 decimal places.");
        setPending("quote");
        const r = await api<QuoteView>(`/api/vesting/${v.id}/quote`, "POST", { amountZec: amount.trim() });
        setPending(undefined);
        if (!r.ok || !r.data) return setError(r.error);
        setQuote(r.data);
      }}
    >
      <Field label="Amount (ZEC)" hint={`Up to ${compactZec(d.withdrawable_zec ?? "0")} ZEC is available now.`}>
        <input
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          required
          inputMode="decimal"
          pattern="[0-9]+(\.[0-9]{1,8})?"
          className={cx(inputClass, "mono")}
          aria-label="Amount in ZEC"
        />
      </Field>
      {error && <Alert>{error}</Alert>}
      <Button type="submit" disabled={pending !== undefined}>
        {pending === "quote" ? "Checking the fee…" : "Review withdrawal"}
      </Button>
    </form>
  );
}
