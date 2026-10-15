"use client";

import { useRouter } from "next/navigation";
import { useState, type ReactNode } from "react";
import { api } from "@/lib/client-api";
import { Alert, Button, Field, inputClass } from "./ui";

/** Shown after any link request; never says whether the address has an account. */
function Sent({ email, children, onBack }: { email: string; children: ReactNode; onBack: () => void }) {
  return (
    <div className="space-y-3">
      <Alert tone="green">
        <span className="font-medium">Check your email.</span> {children} It goes to <span className="font-medium">{email}</span> and expires in 15 minutes.
      </Alert>
      <button type="button" onClick={onBack} className="text-sm text-ink-soft underline hover:text-ink">
        Use a different email
      </button>
    </div>
  );
}

function useRequest() {
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const [sentTo, setSentTo] = useState<string>();
  const send = async (path: string, body: Record<string, string>) => {
    setPending(true);
    setError(undefined);
    const r = await api(path, "POST", body);
    setPending(false);
    if (!r.ok) return setError(r.error);
    setSentTo(body.email);
  };
  return { error, pending, sentTo, send, reset: () => setSentTo(undefined) };
}

/** Login page: "Email me a sign-in link". */
export function EmailSignIn() {
  const { error, pending, sentTo, send, reset } = useRequest();
  if (sentTo) {
    return (
      <Sent email={sentTo} onBack={reset}>
        If an account uses this address, a sign-in link is on its way.
      </Sent>
    );
  }
  return (
    <form
      className="space-y-3"
      onSubmit={(e) => {
        e.preventDefault();
        const email = String(new FormData(e.currentTarget).get("email") ?? "").trim();
        void send("/api/auth/email/start", { mode: "login", email });
      }}
    >
      <Field label="Email">
        <input name="email" type="email" required maxLength={254} autoComplete="email" className={inputClass} />
      </Field>
      {error && <Alert>{error}</Alert>}
      <Button type="submit" disabled={pending} className="w-full">
        {pending ? "Sending…" : "Email me a sign-in link"}
      </Button>
    </form>
  );
}

/** Sign-up page: create an employer account by confirming an e-mail address. */
export function EmailSignUp({ defaultName = "", onNameChange }: { defaultName?: string; onNameChange?: (name: string) => void } = {}) {
  const { error, pending, sentTo, send, reset } = useRequest();
  if (sentTo) {
    return (
      <Sent email={sentTo} onBack={reset}>
        Open the link we sent to finish creating your account.
      </Sent>
    );
  }
  return (
    <form
      className="space-y-3"
      onSubmit={(e) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        void send("/api/auth/email/start", {
          mode: "signup",
          displayName: String(f.get("displayName") ?? "").trim(),
          email: String(f.get("email") ?? "").trim(),
        });
      }}
    >
      <Field label="Your name" hint="Shown to the people you pay or who pay you.">
        <input
          name="displayName"
          required
          maxLength={100}
          defaultValue={defaultName}
          onChange={(e) => onNameChange?.(e.target.value)}
          autoComplete="name"
          className={inputClass}
        />
      </Field>
      <Field label="Email">
        <input name="email" type="email" required maxLength={254} autoComplete="email" className={inputClass} />
      </Field>
      {error && <Alert>{error}</Alert>}
      <Button type="submit" disabled={pending} className="w-full">
        {pending ? "Sending…" : "Email me a sign-up link"}
      </Button>
    </form>
  );
}

/** Employer home: add or change the e-mail used for sign-in links. */
export function BackupEmailForm({ current }: { current: string | null }) {
  const { error, pending, sentTo, send, reset } = useRequest();
  if (sentTo) {
    return (
      <Sent email={sentTo} onBack={reset}>
        Open the confirmation link we sent to start using this address.
      </Sent>
    );
  }
  return (
    <form
      className="flex flex-col gap-3 sm:flex-row sm:items-end"
      onSubmit={(e) => {
        e.preventDefault();
        const email = String(new FormData(e.currentTarget).get("email") ?? "").trim();
        void send("/api/account/email", { email });
      }}
    >
      <div className="min-w-0 flex-1">
        <Field label={current ? "Change sign-in email" : "Sign-in email"}>
          <input name="email" type="email" required maxLength={254} autoComplete="email" placeholder={current ?? "you@company.com"} className={inputClass} />
        </Field>
      </div>
      <Button type="submit" variant="secondary" disabled={pending}>
        {pending ? "Sending…" : "Send confirmation link"}
      </Button>
      {error && (
        <div className="sm:basis-full">
          <Alert>{error}</Alert>
        </div>
      )}
    </form>
  );
}

const ACTION = {
  signup: { title: "Create your account", button: "Create account" },
  login: { title: "Sign in", button: "Sign in" },
  link: { title: "Confirm your sign-in email", button: "Confirm email" },
} as const;

/** The e-mailed link's landing page: one click uses the link (mail scanners only GET it). */
export function EmailLinkConfirm({ token, purpose, email }: { token: string; purpose: keyof typeof ACTION; email: string }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const [linked, setLinked] = useState(false);

  if (linked) {
    return (
      <div className="space-y-4">
        <Alert tone="green">
          <span className="font-medium">{email}</span> is now your sign-in email. You can use it to get a sign-in link if you ever lose your passkey.
        </Alert>
        <Button className="w-full" onClick={() => router.push("/app")}>
          Go to dashboard
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">{ACTION[purpose].title}</h1>
        <p className="mt-1 text-sm text-ink-soft">
          {purpose === "link" ? "Use " : "Continue as "}
          <span className="font-medium text-ink">{email}</span>
          {purpose === "link" ? " for sign-in links." : "."}
        </p>
      </div>
      {error && <Alert>{error}</Alert>}
      <Button
        className="w-full"
        disabled={pending}
        onClick={async () => {
          setPending(true);
          setError(undefined);
          const r = await api<{ purpose: string }>("/api/auth/email/verify", "POST", { token });
          if (!r.ok) {
            setPending(false);
            return setError(r.error);
          }
          if (purpose === "link") {
            setPending(false);
            return setLinked(true);
          }
          router.replace("/app");
          router.refresh();
        }}
      >
        {pending ? "One moment…" : ACTION[purpose].button}
      </Button>
    </div>
  );
}
