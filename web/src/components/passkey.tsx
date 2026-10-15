"use client";

import { browserSupportsWebAuthn, startAuthentication, startRegistration } from "@simplewebauthn/browser";
import { useRouter } from "next/navigation";
import { useState, useSyncExternalStore } from "react";
import { api } from "@/lib/client-api";
import { Alert, Button, Field, inputClass } from "./ui";

function passkeyError(e: unknown): string {
  const name = (e as { name?: string })?.name;
  if (name === "NotAllowedError" || name === "AbortError") return "The passkey prompt was closed. Try again when you're ready.";
  if (name === "InvalidStateError") return ALREADY_HAS_PASSKEY;
  return "Your device couldn't complete the passkey step. Try again.";
}

/** passkeyError() for a device that already holds a hushush passkey (shown with a sign-in link). */
export const ALREADY_HAS_PASSKEY = "This device already has a hushush passkey. Sign in instead.";

const noSubscribe = () => () => {};

/** WebAuthn support (assumed during server rendering). */
export function useWebAuthnSupport(): boolean {
  return useSyncExternalStore(noSubscribe, browserSupportsWebAuthn, () => true);
}

/** After signing in from a worker link: bind the stream and open it. */
async function joinAndOpen(token: string): Promise<{ ok: true; path: string } | { ok: false; error: string }> {
  const r = await api<{ streamId: string }>(`/api/join/${token}`);
  if (r.ok && r.data) return { ok: true, path: `/app/streams/${r.data.streamId}` };
  return { ok: false, error: r.error ?? "Couldn't join this stream." };
}

export type PasskeyStatus = "idle" | "wait" | "done";

/** Passkey sign-up: options, the device prompt, verify, then /app (or the worker link's stream). */
export function usePasskeySignUp({ joinToken }: { joinToken?: string } = {}) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [status, setStatus] = useState<PasskeyStatus>("idle");
  const fail = (message?: string) => {
    setStatus("idle");
    setError(message);
  };
  const start = async (displayName: string) => {
    setStatus("wait");
    setError(undefined);
    const opts = await api<Parameters<typeof startRegistration>[0]["optionsJSON"]>("/api/auth/passkey/register/options", "POST", {
      displayName,
    });
    if (!opts.ok || !opts.data) return fail(opts.error);
    let response;
    try {
      response = await startRegistration({ optionsJSON: opts.data });
    } catch (err) {
      return fail(passkeyError(err));
    }
    const v = await api("/api/auth/passkey/register/verify", "POST", { response });
    if (!v.ok) return fail(v.error);
    if (joinToken) {
      const j = await joinAndOpen(joinToken);
      if (!j.ok) return fail(j.error);
      setStatus("done");
      router.push(j.path);
    } else {
      setStatus("done");
      router.push("/app");
    }
    router.refresh();
  };
  return { status, error, start };
}

/** Passkey sign-in: options, the device prompt, verify, then `next` (or the worker link's stream). */
export function usePasskeySignIn({ next = "/app", joinToken }: { next?: string; joinToken?: string } = {}) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [status, setStatus] = useState<PasskeyStatus>("idle");
  const fail = (message?: string) => {
    setStatus("idle");
    setError(message);
  };
  const start = async () => {
    setStatus("wait");
    setError(undefined);
    const opts = await api<Parameters<typeof startAuthentication>[0]["optionsJSON"]>("/api/auth/passkey/login/options");
    if (!opts.ok || !opts.data) return fail(opts.error);
    let response;
    try {
      response = await startAuthentication({ optionsJSON: opts.data });
    } catch (err) {
      return fail(passkeyError(err));
    }
    const v = await api("/api/auth/passkey/login/verify", "POST", { response });
    if (!v.ok) return fail(v.error);
    if (joinToken) {
      const j = await joinAndOpen(joinToken);
      if (!j.ok) return fail(j.error);
      setStatus("done");
      router.push(j.path);
    } else {
      setStatus("done");
      router.push(next);
    }
    router.refresh();
  };
  return { status, error, start };
}

/** Sign-up with a passkey: name, then Face ID / fingerprint / Windows Hello. (Worker-link page.) */
export function PasskeySignUp({ defaultName = "", joinToken }: { defaultName?: string; joinToken?: string }) {
  const supported = useWebAuthnSupport();
  const { status, error, start } = usePasskeySignUp({ joinToken });

  if (!supported) return <Alert>This browser doesn&apos;t support passkeys. Use a current Chrome, Safari, Edge or Firefox.</Alert>;

  return (
    <form
      className="space-y-4"
      onSubmit={(e) => {
        e.preventDefault();
        void start(String(new FormData(e.currentTarget).get("displayName") ?? "").trim());
      }}
    >
      <Field label="Your name" hint={joinToken ? "Shown to the company paying you." : "Shown to the people you pay or who pay you."}>
        <input name="displayName" required maxLength={100} defaultValue={defaultName} autoComplete="name" className={inputClass} />
      </Field>
      {error && <Alert>{error}</Alert>}
      <Button type="submit" disabled={status !== "idle"} className="w-full">
        {status !== "idle" ? "Waiting for your passkey…" : "Sign up with a passkey"}
      </Button>
      <p className="text-xs text-ink-soft">
        Your device creates a passkey protected by Face ID, your fingerprint, Windows Hello or your device PIN. No password.
      </p>
    </form>
  );
}

/** Sign-in with an existing passkey. (Worker-link page.) */
export function PasskeySignIn({
  next = "/app",
  joinToken,
  label = "Sign in with a passkey",
  variant = "primary",
}: {
  next?: string;
  joinToken?: string;
  label?: string;
  variant?: "primary" | "secondary";
}) {
  const supported = useWebAuthnSupport();
  const { status, error, start } = usePasskeySignIn({ next, joinToken });

  if (!supported) return <Alert>This browser doesn&apos;t support passkeys. Use a current Chrome, Safari, Edge or Firefox.</Alert>;

  return (
    <div className="space-y-3">
      {error && <Alert>{error}</Alert>}
      <Button variant={variant} disabled={status !== "idle"} className="w-full" onClick={() => void start()}>
        {status !== "idle" ? "Waiting for your passkey…" : label}
      </Button>
    </div>
  );
}
