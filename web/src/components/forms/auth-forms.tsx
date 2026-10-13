"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState, type FormEvent } from "react";
import { api } from "@/lib/client-api";
import { Alert, Button, Field, inputClass } from "../ui";

function formValues(e: FormEvent<HTMLFormElement>): Record<string, string> {
  return Object.fromEntries(new FormData(e.currentTarget).entries()) as Record<string, string>;
}

export function SignupForm({ next, email }: { next: string; email?: string }) {
  const [error, setError] = useState<string>();
  const [done, setDone] = useState<string>();
  const [pending, setPending] = useState(false);

  if (done) {
    return (
      <div className="space-y-4">
        <Alert tone="green">
          We sent a verification link to <strong>{done}</strong>. Open it to activate your account, then sign in.
        </Alert>
        <p className="text-sm text-ink-soft">
          Already verified? <Link className="font-medium text-ink underline" href={`/login?next=${encodeURIComponent(next)}`}>Sign in</Link>
        </p>
      </div>
    );
  }

  return (
    <form
      className="space-y-4"
      onSubmit={async (e) => {
        e.preventDefault();
        const v = formValues(e);
        setPending(true);
        setError(undefined);
        const r = await api("/api/auth/signup", "POST", { email: v.email, displayName: v.displayName, password: v.password });
        setPending(false);
        if (r.ok) setDone(v.email);
        else setError(r.error);
      }}
    >
      <Field label="Your name">
        <input name="displayName" required maxLength={100} autoComplete="name" className={inputClass} />
      </Field>
      <Field label="Email">
        <input name="email" type="email" required defaultValue={email} autoComplete="email" className={inputClass} />
      </Field>
      <Field label="Password" hint="At least 12 characters. A passphrase works well.">
        <input name="password" type="password" required minLength={12} maxLength={256} autoComplete="new-password" className={inputClass} />
      </Field>
      {error && <Alert>{error}</Alert>}
      <Button type="submit" disabled={pending} className="w-full">
        {pending ? "Creating account…" : "Create account"}
      </Button>
    </form>
  );
}

export function LoginForm({ next, notice }: { next: string; notice?: string }) {
  const router = useRouter();
  const [error, setError] = useState<{ message: string; code?: string; email?: string }>();
  const [resent, setResent] = useState(false);
  const [pending, setPending] = useState(false);

  return (
    <form
      className="space-y-4"
      onSubmit={async (e) => {
        e.preventDefault();
        const v = formValues(e);
        setPending(true);
        setError(undefined);
        const r = await api("/api/auth/login", "POST", { email: v.email, password: v.password });
        if (r.ok) {
          router.push(next);
          router.refresh();
          return;
        }
        setPending(false);
        setError({ message: r.error ?? "Sign-in failed.", code: r.code, email: v.email });
      }}
    >
      {notice && <Alert tone="green">{notice}</Alert>}
      <Field label="Email">
        <input name="email" type="email" required autoComplete="email" className={inputClass} />
      </Field>
      <Field label="Password">
        <input name="password" type="password" required autoComplete="current-password" className={inputClass} />
      </Field>
      {error && (
        <Alert>
          {error.message}
          {error.code === "email_not_verified" && !resent && (
            <button
              type="button"
              className="ml-2 font-medium underline"
              onClick={async () => {
                await api("/api/auth/resend", "POST", { email: error.email });
                setResent(true);
              }}
            >
              Resend link
            </button>
          )}
          {resent && <span className="ml-2">A new link is on its way.</span>}
        </Alert>
      )}
      <Button type="submit" disabled={pending} className="w-full">
        {pending ? "Signing in…" : "Sign in"}
      </Button>
    </form>
  );
}

export function VerifyEmail({ token }: { token: string }) {
  const [state, setState] = useState<"idle" | "pending" | "ok" | "error">("idle");
  const [error, setError] = useState<string>();
  if (state === "ok") {
    return (
      <div className="space-y-4">
        <Alert tone="green">Your email is verified.</Alert>
        <Link href="/login" className="inline-flex min-h-10 items-center rounded-xl bg-ink px-4 text-sm font-medium text-surface">
          Continue to sign in
        </Link>
      </div>
    );
  }
  return (
    <div className="space-y-4">
      <p className="text-sm text-ink-soft">Confirm that this is your email address to activate your account.</p>
      {error && <Alert>{error}</Alert>}
      <Button
        disabled={state === "pending" || !token}
        onClick={async () => {
          setState("pending");
          const r = await api("/api/auth/verify", "POST", { token });
          if (r.ok) setState("ok");
          else {
            setState("error");
            setError(r.error);
          }
        }}
      >
        {state === "pending" ? "Verifying…" : "Verify my email"}
      </Button>
    </div>
  );
}
