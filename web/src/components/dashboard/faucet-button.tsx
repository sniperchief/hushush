"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { api } from "@/lib/client-api";

/** Testnet: tops the company's payout wallet up from the shared treasury. */
export function FaucetButton({
  available,
  amountZec,
  nextAt,
  remaining,
  compact = false,
}: {
  available: boolean;
  amountZec: string;
  nextAt: string | null;
  remaining: number;
  /** Small button for the wallet metric card; explanations move to its tooltip. */
  compact?: boolean;
}) {
  const router = useRouter();
  // One key per top-up attempt: a double tap or a retry reuses it.
  const [key, setKey] = useState(() => crypto.randomUUID());
  const [pending, setPending] = useState(false);
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string>();

  const note = sent
    ? `${amountZec} test TAZ is on its way. It becomes spendable after a few confirmations (a few minutes).`
    : !available && remaining === 0
      ? "This company has used all its test TAZ top-ups."
      : !available && nextAt
        ? `Next top-up available ${nextAt}.`
        : `Sends ${amountZec} test TAZ from the shared testnet treasury. ${remaining} ${remaining === 1 ? "top-up" : "top-ups"} left.`;

  return (
    <div className="flex flex-col gap-2">
      <button
        type="button"
        disabled={!available || pending || sent}
        onClick={async () => {
          setPending(true);
          setError(undefined);
          const r = await api<{ state: string; failure_reason: string | null }>("/api/company/faucet", "POST", { idempotencyKey: key });
          setPending(false);
          if (!r.ok || !r.data) return setError(r.error);
          if (r.data.state === "failed") {
            setKey(crypto.randomUUID());
            return setError(`The treasury couldn't send test TAZ right now: ${r.data.failure_reason ?? "unknown reason"}.`);
          }
          setSent(true);
          setKey(crypto.randomUUID());
          router.refresh();
        }}
        title={compact ? note : undefined}
        className={`btn-amber cursor-pointer self-start rounded-lg font-semi disabled:cursor-not-allowed disabled:bg-cream/15 disabled:text-oat disabled:shadow-none ${
          compact ? "h-8 px-3 text-[13px]" : "h-10 px-4 text-sm"
        }`}
      >
        {pending ? "Sending…" : sent ? "Test TAZ sent ✓" : "Get test TAZ"}
      </button>
      {/* Compact: only say something when it matters (sent, or why it's unavailable). */}
      {(!compact || sent || !available) && (
        <p className="m-0 text-[12.5px] leading-normal text-oat" role="status">
          {note}
        </p>
      )}
      {error && (
        <p role="alert" className="m-0 rounded-lg border border-failed-line bg-failed-bg px-3 py-2 text-[13px] text-failed-ink">
          {error}
        </p>
      )}
    </div>
  );
}
