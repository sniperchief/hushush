"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api } from "@/lib/client-api";
import { RATE_PERIODS } from "@/lib/periods";
import { Alert, Button, Field, inputClass } from "../ui";

/** Re-renders server data periodically so server-computed balances stay fresh. */
export function AutoRefresh({ seconds = 30 }: { seconds?: number }) {
  const router = useRouter();
  useEffect(() => {
    const t = setInterval(() => router.refresh(), seconds * 1000);
    return () => clearInterval(t);
  }, [router, seconds]);
  return null;
}

export function RefreshButton() {
  const router = useRouter();
  return (
    <Button variant="secondary" onClick={() => router.refresh()}>
      Refresh
    </Button>
  );
}

/** datetime-local value → unix seconds (interpreted in the browser's time zone). */
function toUnix(v: FormDataEntryValue | null): number | null {
  if (!v) return null;
  const ms = new Date(String(v)).getTime();
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
}

export function CreateStreamForm({ orgId, members }: { orgId: string; members: { id: string; label: string }[] }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const [startMode, setStartMode] = useState<"now" | "later">("now");

  if (members.length === 0) {
    return <Alert tone="neutral">Invite a contributor first. Streams can only pay members of this organization.</Alert>;
  }

  return (
    <form
      className="grid gap-4 sm:grid-cols-2"
      onSubmit={async (e) => {
        e.preventDefault();
        const fd = new FormData(e.currentTarget);
        const startAt = startMode === "now" ? null : toUnix(fd.get("startAt"));
        if (startMode === "later" && startAt === null) {
          setError("Choose a start time.");
          return;
        }
        setPending(true);
        setError(undefined);
        const r = await api<{ id: string }>(`/api/orgs/${orgId}/streams`, "POST", {
          recipientUserId: fd.get("recipient"),
          rateZec: String(fd.get("rateZec") ?? "").trim(),
          ratePeriodSecs: Number(fd.get("period")),
          startAt,
          endAt: toUnix(fd.get("endAt")),
        });
        if (r.ok && r.data) {
          router.push(`/app/streams/${r.data.id}`);
          router.refresh();
        } else {
          setPending(false);
          setError(r.error);
        }
      }}
    >
      <Field label="Contributor">
        <select name="recipient" required className={inputClass}>
          {members.map((m) => (
            <option key={m.id} value={m.id}>{m.label}</option>
          ))}
        </select>
      </Field>
      <div className="grid grid-cols-[1fr_auto] gap-2">
        <Field label="Rate (ZEC)" hint="Fixed for the life of the stream.">
          <input name="rateZec" required inputMode="decimal" pattern="^\d{1,8}(\.\d{1,8})?$" placeholder="1.5" className={`${inputClass} font-mono`} />
        </Field>
        <Field label="Period">
          <select name="period" defaultValue={2592000} className={inputClass}>
            {RATE_PERIODS.map((p) => (
              <option key={p.secs} value={p.secs}>{p.label}</option>
            ))}
          </select>
        </Field>
      </div>
      <Field label="Start" hint="Streams can't start in the past.">
        <div className="flex gap-2">
          <select aria-label="Start mode" value={startMode} onChange={(e) => setStartMode(e.target.value as "now" | "later")} className={`${inputClass} w-auto`}>
            <option value="now">Now</option>
            <option value="later">Later…</option>
          </select>
          {startMode === "later" && <input name="startAt" type="datetime-local" required className={inputClass} />}
        </div>
      </Field>
      <Field label="End (optional)" hint="Leave empty for an open-ended stream. Times use your browser's time zone.">
        <input name="endAt" type="datetime-local" className={inputClass} />
      </Field>
      {error && <div className="sm:col-span-2"><Alert>{error}</Alert></div>}
      <div className="sm:col-span-2">
        <Button type="submit" disabled={pending}>{pending ? "Creating…" : "Create stream"}</Button>
      </div>
    </form>
  );
}

export function StreamActions({ streamId, status }: { streamId: string; status: "active" | "paused" | "ended" }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  if (status === "ended") return null;

  async function run(action: "pause" | "resume" | "end") {
    if (action === "end" && !confirm("End this stream permanently? Wages already earned stay claimable. To change the rate, end it and create a new stream.")) return;
    setPending(true);
    setError(undefined);
    const r = await api(`/api/streams/${streamId}/lifecycle/${action}`);
    setPending(false);
    if (!r.ok) setError(r.error);
    router.refresh();
  }

  return (
    <div className="flex flex-wrap items-center gap-2">
      {status === "active" ? (
        <Button variant="secondary" disabled={pending} onClick={() => run("pause")}>Pause</Button>
      ) : (
        <Button variant="secondary" disabled={pending} onClick={() => run("resume")}>Resume</Button>
      )}
      <Button variant="danger" disabled={pending} onClick={() => run("end")}>End stream</Button>
      {error && <p className="w-full text-sm text-bad">{error}</p>}
    </div>
  );
}

function newKey(): string {
  return crypto.randomUUID();
}

/**
 * Claim request. One idempotency key per intended claim: retries and double
 * submits reuse it, so the server returns the original reservation instead of
 * creating a second one. A fresh key is minted only after a definite outcome.
 */
export function ClaimForm({ streamId, availableZec, availableZats }: { streamId: string; availableZec: string; availableZats: number }) {
  const router = useRouter();
  const [key, setKey] = useState(newKey);
  const [amount, setAmount] = useState("");
  const [msg, setMsg] = useState<{ tone: "red" | "green" | "amber"; text: string }>();
  const [pending, setPending] = useState(false);

  return (
    <form
      className="space-y-3"
      onSubmit={async (e) => {
        e.preventDefault();
        if (pending) return;
        setPending(true);
        setMsg(undefined);
        const r = await api<{ duplicate: boolean; claim: { amount_zec: string } }>(`/api/streams/${streamId}/claims`, "POST", {
          amountZec: amount.trim(),
          idempotencyKey: key,
        });
        setPending(false);
        if (r.ok && r.data) {
          setMsg({
            tone: "green",
            text: r.data.duplicate
              ? "This claim was already submitted; showing the original reservation."
              : `Reserved ${r.data.claim.amount_zec} ZEC. It's pending and unpaid until you pay it out.`,
          });
          setAmount("");
          setKey(newKey());
          router.refresh();
        } else if (r.status === 0 || r.status >= 500) {
          // Outcome unknown: keep the same key so a retry can't double-reserve.
          setMsg({ tone: "amber", text: `${r.error} Submitting again is safe; it won't create a duplicate claim.` });
        } else {
          setMsg({ tone: "red", text: r.error ?? "Claim failed." });
          setKey(newKey());
        }
      }}
    >
      <Field label="Amount to claim (ZEC)" hint={`Available now: ${availableZec} ZEC`}>
        <div className="flex gap-2">
          <input
            required
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            inputMode="decimal"
            pattern="^\d{1,8}(\.\d{1,8})?$"
            placeholder="0.25"
            className={`${inputClass} font-mono`}
          />
          <Button type="button" variant="secondary" disabled={availableZats <= 0} onClick={() => setAmount(availableZec)}>
            Full amount
          </Button>
        </div>
      </Field>
      {msg && <Alert tone={msg.tone}>{msg.text}</Alert>}
      <Button type="submit" disabled={pending || availableZats <= 0}>{pending ? "Reserving…" : "Request claim"}</Button>
      <p className="text-xs text-ink-soft">A claim reserves earned wages. Nothing is sent until you choose &ldquo;Pay out&rdquo; on the claim.</p>
    </form>
  );
}

export function ReleaseClaimButton({ claimId }: { claimId: string }) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  return (
    <Button
      variant="ghost"
      disabled={pending}
      onClick={async () => {
        if (!confirm("Cancel this claim? The amount becomes available again.")) return;
        setPending(true);
        await api(`/api/claims/${claimId}/release`);
        router.refresh();
      }}
    >
      Cancel
    </Button>
  );
}
