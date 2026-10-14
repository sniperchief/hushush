"use client";

import { browserSupportsWebAuthn, startAuthentication, startRegistration } from "@simplewebauthn/browser";
import { useRouter } from "next/navigation";
import { useState, useSyncExternalStore } from "react";
import { api } from "@/lib/client-api";
import { Alert, Button, Field, inputClass } from "./ui";

type Kind = "employer" | "worker";

function passkeyError(e: unknown): string {
  const name = (e as { name?: string })?.name;
  if (name === "NotAllowedError" || name === "AbortError") return "The passkey prompt was closed. Try again when you're ready.";
  if (name === "InvalidStateError") return "This device already has a hushush passkey. Sign in instead.";
  return "Your device couldn't complete the passkey step. Try again.";
}

const noSubscribe = () => () => {};

/** WebAuthn support (assumed during server rendering). */
function useWebAuthnSupport(): boolean {
  return useSyncExternalStore(noSubscribe, browserSupportsWebAuthn, () => true);
}

/** After signing in from a worker link: bind the stream and open it. */
async function joinAndOpen(token: string): Promise<{ ok: true; path: string } | { ok: false; error: string }> {
  const r = await api<{ streamId: string }>(`/api/join/${token}`);
  if (r.ok && r.data) return { ok: true, path: `/app/streams/${r.data.streamId}` };
  return { ok: false, error: r.error ?? "Couldn't join this stream." };
}

/** Sign-up with a passkey: name, then Face ID / fingerprint / Windows Hello. */
export function PasskeySignUp({ kind, defaultName = "", joinToken }: { kind: Kind; defaultName?: string; joinToken?: string }) {
  const router = useRouter();
  const supported = useWebAuthnSupport();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);

  if (!supported) return <Alert>This browser doesn&apos;t support passkeys. Use a current Chrome, Safari, Edge or Firefox.</Alert>;

  return (
    <form
      className="space-y-4"
      onSubmit={async (e) => {
        e.preventDefault();
        const displayName = String(new FormData(e.currentTarget).get("displayName") ?? "").trim();
        setPending(true);
        setError(undefined);
        const opts = await api<Parameters<typeof startRegistration>[0]["optionsJSON"]>("/api/auth/passkey/register/options", "POST", {
          kind,
          displayName,
        });
        if (!opts.ok || !opts.data) {
          setPending(false);
          return setError(opts.error);
        }
        let response;
        try {
          response = await startRegistration({ optionsJSON: opts.data });
        } catch (err) {
          setPending(false);
          return setError(passkeyError(err));
        }
        const v = await api("/api/auth/passkey/register/verify", "POST", { response });
        if (!v.ok) {
          setPending(false);
          return setError(v.error);
        }
        if (joinToken) {
          const j = await joinAndOpen(joinToken);
          if (!j.ok) {
            setPending(false);
            return setError(j.error);
          }
          router.push(j.path);
        } else {
          router.push("/app");
        }
        router.refresh();
      }}
    >
      <Field label="Your name" hint={kind === "worker" ? "Shown to the company paying you." : "Shown to the people you pay."}>
        <input name="displayName" required maxLength={100} defaultValue={defaultName} autoComplete="name" className={inputClass} />
      </Field>
      {error && <Alert>{error}</Alert>}
      <Button type="submit" disabled={pending} className="w-full">
        {pending ? "Waiting for your passkey…" : "Sign up with a passkey"}
      </Button>
      <p className="text-xs text-ink-soft">
        Your device creates a passkey protected by Face ID, your fingerprint, Windows Hello or your device PIN. No password, no e-mail.
      </p>
    </form>
  );
}

/** Sign-in with an existing passkey. */
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
  const router = useRouter();
  const supported = useWebAuthnSupport();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);

  if (!supported) return <Alert>This browser doesn&apos;t support passkeys. Use a current Chrome, Safari, Edge or Firefox.</Alert>;

  return (
    <div className="space-y-3">
      {error && <Alert>{error}</Alert>}
      <Button
        variant={variant}
        disabled={pending}
        className="w-full"
        onClick={async () => {
          setPending(true);
          setError(undefined);
          const opts = await api<Parameters<typeof startAuthentication>[0]["optionsJSON"]>("/api/auth/passkey/login/options");
          if (!opts.ok || !opts.data) {
            setPending(false);
            return setError(opts.error);
          }
          let response;
          try {
            response = await startAuthentication({ optionsJSON: opts.data });
          } catch (err) {
            setPending(false);
            return setError(passkeyError(err));
          }
          const v = await api<{ kind: Kind }>("/api/auth/passkey/login/verify", "POST", { response });
          if (!v.ok || !v.data) {
            setPending(false);
            return setError(v.error);
          }
          if (joinToken) {
            if (v.data.kind !== "worker") {
              // Signed in, but as an employer: the join page explains what to do.
              router.refresh();
              return setPending(false);
            }
            const j = await joinAndOpen(joinToken);
            if (!j.ok) {
              setPending(false);
              return setError(j.error);
            }
            router.push(j.path);
          } else {
            router.push(next);
          }
          router.refresh();
        }}
      >
        {pending ? "Waiting for your passkey…" : label}
      </Button>
    </div>
  );
}
