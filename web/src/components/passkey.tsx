"use client";

import { browserSupportsWebAuthn, startAuthentication, startRegistration } from "@simplewebauthn/browser";
import { useRouter } from "next/navigation";
import { useState, useSyncExternalStore } from "react";
import { api } from "@/lib/client-api";

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
