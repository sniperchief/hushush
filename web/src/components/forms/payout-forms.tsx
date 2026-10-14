"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { api } from "@/lib/client-api";
import { compactZec } from "@/lib/format";
import type { PayoutQuote, PayoutView } from "@/lib/payroll";
import { Alert, Button, Field, inputClass } from "../ui";

function short(s: string, n = 10): string {
  return s.length <= 2 * n + 1 ? s : `${s.slice(0, n)}…${s.slice(-n)}`;
}

type Msg = { tone: "red" | "green" | "amber"; text: string };

/**
 * Deliberate two-step payout of a reserved claim: review (address checked,
 * real network fee quoted) → send. Sending is idempotent per claim on the
 * server, so a retry after a network error can never pay twice.
 */
export function PayoutForm({ claimId, amountZec, onClose }: { claimId: string; amountZec: string; onClose: () => void }) {
  const router = useRouter();
  const [address, setAddress] = useState("");
  const [quote, setQuote] = useState<PayoutQuote>();
  const [pending, setPending] = useState(false);
  const [msg, setMsg] = useState<Msg>();

  async function review(e: React.FormEvent) {
    e.preventDefault();
    if (pending) return;
    setPending(true);
    setMsg(undefined);
    const r = await api<PayoutQuote>(`/api/claims/${claimId}/payout/quote`, "POST", { address: address.trim() });
    setPending(false);
    if (!r.ok || !r.data) return setMsg({ tone: "red", text: r.error ?? "Could not check this payout." });
    if (!r.data.valid_recipient) return setMsg({ tone: "red", text: r.data.reason ?? "This address can't receive payouts." });
    setQuote(r.data);
  }

  async function send() {
    if (pending) return;
    setPending(true);
    setMsg(undefined);
    const r = await api<{ payout: PayoutView; duplicate: boolean }>(`/api/claims/${claimId}/payout`, "POST", { address: address.trim() });
    setPending(false);
    if (r.ok && r.data) {
      const p = r.data.payout;
      if (p.state === "failed") {
        setMsg({ tone: "red", text: `The payout was not sent: ${p.detail ?? "unknown reason"}. The amount is available to claim again.` });
      } else if (p.state === "requires_reconciliation" || p.state === "reserved") {
        setMsg({ tone: "amber", text: "We couldn't confirm the outcome yet. It is being checked automatically and will never be paid twice." });
      } else {
        setMsg({ tone: "green", text: `Sent to the network. It counts as paid after ${p.required_confirmations} confirmations.` });
      }
      router.refresh();
    } else if (r.status === 0 || r.status >= 500) {
      setMsg({ tone: "amber", text: `${r.error} Pressing Send again is safe: this claim can only ever be paid once.` });
    } else {
      setMsg({ tone: "red", text: r.error ?? "Payout failed." });
      router.refresh();
    }
  }

  return (
    <div className="mt-3 w-full space-y-3 rounded-xl border border-line bg-muted/40 p-4">
      {!quote ? (
        <form className="space-y-3" onSubmit={review}>
          <Field label="Your shielded Zcash testnet address" hint="A Unified Address (starts with utest1) with an Orchard receiver, e.g. from Zashi in testnet mode.">
            <input
              required
              autoComplete="off"
              spellCheck={false}
              value={address}
              onChange={(e) => setAddress(e.target.value)}
              placeholder="utest1…"
              className={`${inputClass} font-mono`}
            />
          </Field>
          {msg && <Alert tone={msg.tone}>{msg.text}</Alert>}
          <div className="flex flex-wrap gap-2">
            <Button type="submit" disabled={pending || address.trim().length < 10}>{pending ? "Checking…" : "Review payout"}</Button>
            <Button type="button" variant="ghost" onClick={onClose}>Cancel</Button>
          </div>
        </form>
      ) : (
        <div className="space-y-3">
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
            <dt className="text-ink-soft">You receive</dt>
            <dd className="font-mono font-medium">{compactZec(amountZec)} ZEC <span className="text-ink-soft">(testnet)</span></dd>
            <dt className="text-ink-soft">Network fee</dt>
            <dd className="font-mono">{quote.fee_zec ? `${compactZec(quote.fee_zec)} ZEC` : "—"} <span className="text-ink-soft">paid by your employer</span></dd>
            <dt className="text-ink-soft">To</dt>
            <dd className="font-mono break-all" title={address.trim()}>{short(address.trim(), 14)}</dd>
            <dt className="text-ink-soft">Paid when</dt>
            <dd>{quote.required_confirmations} block confirmations</dd>
          </dl>
          <p className="text-xs text-ink-soft">
            Sent as a fully shielded transaction: the amount, the addresses and the memo are encrypted. The fee and the time it is sent are public.
          </p>
          {!quote.sufficient && <Alert tone="amber">{quote.reason ?? "The payout wallet can't cover this payout right now. Try again later."}</Alert>}
          {msg && <Alert tone={msg.tone}>{msg.text}</Alert>}
          <div className="flex flex-wrap gap-2">
            <Button onClick={send} disabled={pending || !quote.sufficient || msg?.tone === "green"}>
              {pending ? "Sending… (building a zero-knowledge proof)" : "Send payout"}
            </Button>
            <Button variant="ghost" disabled={pending} onClick={() => { setQuote(undefined); setMsg(undefined); }}>Change address</Button>
            <Button variant="ghost" disabled={pending} onClick={onClose}>Close</Button>
          </div>
        </div>
      )}
    </div>
  );
}

export function PayoutButton({ claimId, amountZec }: { claimId: string; amountZec: string }) {
  const [open, setOpen] = useState(false);
  return open ? (
    <PayoutForm claimId={claimId} amountZec={amountZec} onClose={() => setOpen(false)} />
  ) : (
    <Button variant="secondary" onClick={() => setOpen(true)}>Pay out</Button>
  );
}
