"use client";

import Link from "next/link";
import { useRef, type ReactNode } from "react";
import { AuthAlert, authCard, Fingerprint, hintClass, OrDivider, primaryButton, secondaryButton, textButton, UnderlineField } from "./auth-ui";
import { useLinkRequest } from "./email-login";
import { ALREADY_HAS_PASSKEY, usePasskeySignIn, usePasskeySignUp, useWebAuthnSupport, type PasskeyStatus } from "./passkey";

// Passkey and email side by side: passkey button, "OR", email form. Without
// WebAuthn support only the email form is shown, as the primary action.

function PasskeyError({ error }: { error?: string }) {
  if (!error) return null;
  if (error === ALREADY_HAS_PASSKEY) {
    return (
      <AuthAlert tone="bad">
        This device already has a hushush passkey.{" "}
        <Link href="/login" className="font-semi text-failed-ink underline">
          Sign in instead
        </Link>
        .
      </AuthAlert>
    );
  }
  return <AuthAlert tone="bad">{error}</AuthAlert>;
}

function PasskeyButton({ status, label, done, onClick }: { status: PasskeyStatus; label: string; done: string; onClick: () => void }) {
  return (
    <button type="button" className={primaryButton} disabled={status !== "idle"} onClick={onClick}>
      <Fingerprint waiting={status === "wait"} />
      {status === "wait" ? "Waiting for your passkey…" : status === "done" ? done : label}
    </button>
  );
}

/** Replaces the whole card once a link is on its way. */
function Sent({ children, onBack }: { children: ReactNode; onBack: () => void }) {
  return (
    <div className="flex flex-col gap-3 sm:gap-3.5">
      <AuthAlert tone="ok">
        <b className="font-semi">Check your email.</b> {children}
      </AuthAlert>
      <button type="button" className={`${textButton} self-start`} onClick={onBack}>
        Use a different email
      </button>
    </div>
  );
}

const sentTo = (email: string) => (
  <>
    It goes to <b className="font-semi break-all">{email}</b> and expires in 15 minutes.
  </>
);

export function SignUpPanel() {
  const supported = useWebAuthnSupport();
  const passkey = usePasskeySignUp();
  const link = useLinkRequest();
  const nameRef = useRef<HTMLInputElement>(null);
  const busy = passkey.status !== "idle" || link.pending;

  /** The one name field feeds both actions. */
  const name = (): string | null => {
    const el = nameRef.current;
    if (!el?.reportValidity()) return null;
    return el.value.trim();
  };

  if (link.sentTo) {
    return (
      <section className={authCard}>
        <Sent onBack={link.reset}>Open the link we sent to finish creating your account. {sentTo(link.sentTo)}</Sent>
      </section>
    );
  }

  return (
    <section className={authCard}>
      <div className="flex flex-col gap-2 sm:gap-1.5">
        <UnderlineField id="su-name" label="Your name" ref={nameRef} required maxLength={100} autoComplete="name" placeholder="Ava Nolan" />
        <p className={hintClass}>Shown to the people you pay or who pay you.</p>
      </div>

      {supported && (
        <>
          <div className="flex flex-col gap-2.5 sm:gap-2">
            <PasskeyError error={passkey.error} />
            <PasskeyButton
              status={passkey.status}
              label="Sign up with a passkey"
              done="Account created"
              onClick={() => {
                const n = name();
                if (n) void passkey.start(n);
              }}
            />
            <p className={hintClass}>Uses Face ID, your fingerprint, Windows Hello or device PIN. No password.</p>
          </div>
          <OrDivider />
        </>
      )}

      <form
        className="flex flex-col gap-3.5 sm:gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          const n = name();
          if (!n) return;
          const email = String(new FormData(e.currentTarget).get("email") ?? "").trim();
          void link.send("/api/auth/email/start", { mode: "signup", displayName: n, email });
        }}
      >
        <UnderlineField id="su-email" label="Email" name="email" type="email" required maxLength={254} autoComplete="email" placeholder="you@company.com" />
        {link.error && <AuthAlert tone="bad">{link.error}</AuthAlert>}
        <button type="submit" className={supported ? secondaryButton : primaryButton} disabled={busy}>
          {link.pending ? "Sending…" : "Email me a sign-up link"}
        </button>
      </form>
    </section>
  );
}

export function SignInPanel({ next }: { next: string }) {
  const supported = useWebAuthnSupport();
  const passkey = usePasskeySignIn({ next });
  const link = useLinkRequest();
  const busy = passkey.status !== "idle" || link.pending;

  if (link.sentTo) {
    return (
      <section className={authCard}>
        <Sent onBack={link.reset}>If an account uses this address, a sign-in link is on its way. {sentTo(link.sentTo)}</Sent>
      </section>
    );
  }

  return (
    <section className={authCard}>
      {supported ? (
        <>
          <div className="flex flex-col gap-2.5 sm:gap-2">
            <PasskeyError error={passkey.error} />
            <PasskeyButton status={passkey.status} label="Sign in with a passkey" done="Signed in" onClick={() => void passkey.start()} />
            <p className={`${hintClass} text-center`}>
              <span className="sm:hidden">Face ID, fingerprint or your device PIN.</span>
              <span className="max-sm:hidden">Face ID, fingerprint, Windows Hello or your device PIN.</span>
            </p>
          </div>
          <OrDivider />
        </>
      ) : (
        <AuthAlert tone="info">This browser doesn&apos;t support passkeys. Use a sign-in link instead.</AuthAlert>
      )}

      <form
        className="flex flex-col gap-3.5 sm:gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          const email = String(new FormData(e.currentTarget).get("email") ?? "").trim();
          void link.send("/api/auth/email/start", { mode: "login", email });
        }}
      >
        <UnderlineField id="li-email" label="Email" name="email" type="email" required maxLength={254} autoComplete="email" placeholder="you@company.com" />
        {link.error && <AuthAlert tone="bad">{link.error}</AuthAlert>}
        <button type="submit" className={supported ? secondaryButton : primaryButton} disabled={busy}>
          {link.pending ? "Sending…" : "Email me a sign-in link"}
        </button>
        <p className={hintClass}>Works if you signed up with email or added a sign-in email to your account.</p>
      </form>
    </section>
  );
}
