"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import QRCode from "qrcode";
import { useEffect, useState, useSyncExternalStore } from "react";
import { api } from "@/lib/client-api";
import { AuthAlert, authCard, hintClass, primaryButton, UnderlineField } from "./auth-ui";
import { Alert, Button, Field, inputClass } from "./ui";

/** Step 2 of employer sign-up: the company name. */
export function CompanyForm() {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  return (
    <form
      className={authCard}
      onSubmit={async (e) => {
        e.preventDefault();
        setPending(true);
        setError(undefined);
        const r = await api("/api/company", "POST", { name: String(new FormData(e.currentTarget).get("name") ?? "").trim() });
        setPending(false);
        if (!r.ok) return setError(r.error);
        router.refresh();
      }}
    >
      <div className="flex flex-col gap-2">
        <UnderlineField id="ob-company" label="Company name" name="name" required maxLength={100} autoComplete="organization" placeholder="Kestrel Collective" />
        <p className={hintClass}>Workers see this when you send them a stream.</p>
      </div>
      {error && <AuthAlert tone="bad">{error}</AuthAlert>}
      <button type="submit" disabled={pending} className={primaryButton}>
        {pending ? "Saving…" : "Continue"}
      </button>
    </form>
  );
}

/** The worker link with copy and QR buttons. It is shown only once. */
export function WorkerLink({ url }: { url: string }) {
  const [qr, setQr] = useState<string>();
  const [showQr, setShowQr] = useState(false);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    QRCode.toDataURL(url, { margin: 1, width: 240 }).then(setQr, () => setQr(undefined));
  }, [url]);
  return (
    <div className="space-y-3">
      <div className="flex flex-col gap-2 sm:flex-row">
        <input readOnly value={url} onFocus={(e) => e.currentTarget.select()} aria-label="Worker link" className={`${inputClass} mono text-xs`} />
        <div className="flex gap-2">
          <Button
            type="button"
            variant="secondary"
            onClick={async () => {
              await navigator.clipboard.writeText(url);
              setCopied(true);
              setTimeout(() => setCopied(false), 2000);
            }}
          >
            {copied ? "Copied ✓" : "Copy"}
          </Button>
          <Button type="button" variant="secondary" onClick={() => setShowQr((v) => !v)} disabled={!qr}>
            QR
          </Button>
        </div>
      </div>
      {showQr && qr && (
        // eslint-disable-next-line @next/next/no-img-element -- a generated data: URL
        <img src={qr} alt="QR code of the worker link" width={240} height={240} className="rounded-lg border border-linen bg-paper p-2" />
      )}
      <p className="m-0 text-[12.5px] leading-[1.45] text-bark">
        Send this link to your worker any way you like. It works once: the first person to sign up or sign in through it gets the stream.
        For your security it is shown only now — if you lose it, issue a new one under Streams → Waiting to join.
      </p>
    </div>
  );
}

/** datetime-local value → unix seconds (in the browser's time zone). */
function toUnix(v: FormDataEntryValue | null): number | null {
  if (!v) return null;
  const ms = new Date(String(v)).getTime();
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
}

function localInput(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

const noSubscribe = () => () => {};
/** 30 days from now, to the minute (stable between renders within a minute). */
const defaultEndInput = () => localInput(new Date(Date.now() + 30 * 86_400_000));

export function NewStreamForm() {
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const [startMode, setStartMode] = useState<"now" | "later">("now");
  const [created, setCreated] = useState<{ link: string; worker: string; total: string }>();
  // In the browser's time zone, so only known client-side ("" on the server).
  const defaultEnd = useSyncExternalStore(noSubscribe, defaultEndInput, () => "");

  if (created) {
    return (
      <div className="space-y-4">
        <Alert tone="green">
          Stream created: {created.total} ZEC for {created.worker}. Now send them their link.
        </Alert>
        <WorkerLink url={created.link} />
        <Link href="/app/streams" className="btn-amber inline-flex h-[42px] items-center rounded-lg px-[18px] text-[15px] font-semi no-underline">
          Done
        </Link>
      </div>
    );
  }

  return (
    <form
      className="grid gap-4 sm:grid-cols-2"
      onSubmit={async (e) => {
        e.preventDefault();
        const fd = new FormData(e.currentTarget);
        const startAt = startMode === "now" ? null : toUnix(fd.get("startAt"));
        const endAt = toUnix(fd.get("endAt"));
        if (startMode === "later" && startAt === null) return setError("Choose a start time.");
        if (endAt === null) return setError("Choose an end date.");
        setPending(true);
        setError(undefined);
        const workerName = String(fd.get("workerName") ?? "").trim();
        const totalZec = String(fd.get("totalZec") ?? "").trim();
        const r = await api<{ link: string; offer: { total_zec: string } }>("/api/streams", "POST", { workerName, totalZec, startAt, endAt });
        setPending(false);
        if (!r.ok || !r.data) return setError(r.error);
        setCreated({ link: r.data.link, worker: workerName, total: totalZec });
      }}
    >
      <Field label="Worker's name">
        <input name="workerName" required maxLength={100} className={inputClass} />
      </Field>
      <Field label="Total (ZEC)" hint="Paid out evenly, by the second, between start and end.">
        <input name="totalZec" required inputMode="decimal" placeholder="1.5" pattern="[0-9]+(\.[0-9]{1,8})?" className={`${inputClass} mono`} />
      </Field>
      <Field label="Start">
        <div className="flex gap-2">
          <select value={startMode} onChange={(e) => setStartMode(e.target.value as "now" | "later")} className={inputClass} aria-label="Start">
            <option value="now">Now</option>
            <option value="later">Later…</option>
          </select>
          {startMode === "later" && <input type="datetime-local" name="startAt" required className={inputClass} aria-label="Start time" />}
        </div>
      </Field>
      <Field label="End date">
        <input type="datetime-local" name="endAt" required defaultValue={defaultEnd} key={defaultEnd} className={inputClass} />
      </Field>
      {error && (
        <div className="sm:col-span-2">
          <Alert>{error}</Alert>
        </div>
      )}
      <div className="sm:col-span-2">
        <Button type="submit" disabled={pending}>{pending ? "Creating…" : "Create stream and get worker link"}</Button>
      </div>
    </form>
  );
}

/** Actions on a stream nobody has joined yet: issue a new link, or cancel. */
export function OfferActions({ offerId }: { offerId: string }) {
  const router = useRouter();
  const [link, setLink] = useState<string>();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          className="h-[38px] cursor-pointer rounded-lg border border-espresso bg-transparent px-3.5 text-sm font-semi whitespace-nowrap text-espresso hover:bg-cream disabled:opacity-60"
          disabled={pending}
          onClick={async () => {
            setPending(true);
            setError(undefined);
            const r = await api<{ link: string }>(`/api/offers/${offerId}/link`);
            setPending(false);
            if (!r.ok || !r.data) return setError(r.error);
            setLink(r.data.link);
          }}
        >
          {pending ? "One moment…" : "New worker link"}
        </button>
        <button
          type="button"
          className="h-[38px] cursor-pointer rounded-lg border border-danger-line bg-danger-soft px-3.5 text-sm font-semi text-failed-ink disabled:opacity-60"
          disabled={pending}
          onClick={async () => {
            if (!confirm("Cancel this stream? The worker link stops working. Nothing is owed on it.")) return;
            setPending(true);
            const r = await api(`/api/offers/${offerId}/cancel`);
            setPending(false);
            if (!r.ok) return setError(r.error);
            router.refresh();
          }}
        >
          Cancel
        </button>
      </div>
      {link && (
        <div className="rounded-[10px] border border-linen bg-cream p-3.5">
          <WorkerLink url={link} />
        </div>
      )}
      {error && <Alert>{error}</Alert>}
    </div>
  );
}

export function EndStreamButton({ streamId }: { streamId: string }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  return (
    <div className="space-y-2">
      <Button
        variant="danger"
        disabled={pending}
        onClick={async () => {
          if (!confirm("End this stream now? Earnings stop now; the worker can still withdraw what they've earned.")) return;
          setPending(true);
          const r = await api(`/api/streams/${streamId}/end`);
          setPending(false);
          if (!r.ok) return setError(r.error);
          router.refresh();
        }}
      >
        End stream
      </Button>
      {error && <Alert>{error}</Alert>}
    </div>
  );
}
