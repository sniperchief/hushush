"use client";

import { useRouter } from "next/navigation";
import { useState, type ReactNode } from "react";
import type { Intent } from "@/lib/auth";
import { api } from "@/lib/client-api";
import { Alert } from "./ui";

function useSetIntent() {
  const router = useRouter();
  const [pending, setPending] = useState<Intent>();
  const [error, setError] = useState<string>();
  const choose = async (intent: Intent) => {
    setPending(intent);
    setError(undefined);
    const r = await api("/api/account/intent", "POST", { intent });
    if (!r.ok) {
      setPending(undefined);
      return setError(r.error);
    }
    router.refresh();
  };
  return { pending, error, choose };
}

const OPTIONS: { intent: Intent; title: string; body: string }[] = [
  { intent: "pay", title: "I pay a team", body: "Stream pay to the people who work with you, every second, privately." },
  { intent: "get_paid", title: "I get paid through hushush", body: "See your pay grow as you earn it and withdraw to your own wallet." },
];

/** First screen after sign-up: picks where the account starts. Both stay possible. */
export function IntentQuestion({ name }: { name: string }) {
  const { pending, error, choose } = useSetIntent();
  return (
    <div className="mx-auto max-w-xl space-y-6">
      <div>
        <p className="text-sm text-ink-soft">Welcome, {name}</p>
        <h1 className="mt-1 text-2xl font-semibold tracking-tight">How will you use hushush?</h1>
        <p className="mt-1 text-sm text-ink-soft">Pick where to start. You can do both later.</p>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        {OPTIONS.map((o) => (
          <button
            key={o.intent}
            type="button"
            disabled={pending !== undefined}
            onClick={() => choose(o.intent)}
            className="flex cursor-pointer flex-col gap-2 rounded-2xl border border-line bg-surface p-5 text-left shadow-sm transition hover:border-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-wait disabled:opacity-60"
          >
            <span className="text-base font-semibold tracking-tight">{o.title}</span>
            <span className="text-sm text-ink-soft">{o.body}</span>
            {pending === o.intent && <span className="text-xs text-ink-soft">One moment…</span>}
          </button>
        ))}
      </div>
      {error && <Alert>{error}</Alert>}
    </div>
  );
}

/** Link to the other side of hushush, e.g. a worker who starts paying people. */
export function SwitchIntent({ to, children }: { to: Intent; children: ReactNode }) {
  const { pending, error, choose } = useSetIntent();
  return (
    <span className="inline-flex flex-col items-start">
      <button
        type="button"
        disabled={pending !== undefined}
        onClick={() => choose(to)}
        className="cursor-pointer text-sm text-ink-soft underline underline-offset-2 hover:text-ink disabled:cursor-wait disabled:opacity-60"
      >
        {pending ? "One moment…" : children}
      </button>
      {error && <span className="mt-1 text-xs text-bad">{error}</span>}
    </span>
  );
}
