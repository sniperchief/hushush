"use client";

import { useRouter } from "next/navigation";
import { useRef, useState } from "react";
import { api } from "@/lib/client-api";
import { AuthAlert, authCard, Fingerprint, hintClass, OrDivider, primaryButton, secondaryButton, UnderlineField } from "./auth-ui";
import { ALREADY_HAS_PASSKEY, usePasskeySignIn, usePasskeySignUp, useWebAuthnSupport, type PasskeyStatus } from "./passkey";

// The worker link's account step, in the sign-up page's design. Joining works
// as before: a passkey sign-up or sign-in, then the stream binds to that
// account (see usePasskeySignUp/usePasskeySignIn with `joinToken`).

function PasskeyButton({ status, label, done, onClick, secondary = false }: { status: PasskeyStatus; label: string; done: string; onClick: () => void; secondary?: boolean }) {
  return (
    <button type="button" className={secondary ? secondaryButton : primaryButton} disabled={status !== "idle"} onClick={onClick}>
      <Fingerprint waiting={status === "wait"} />
      {status === "wait" ? "Waiting for your passkey…" : status === "done" ? done : label}
    </button>
  );
}

/** Signed out: create an account (or sign in) with a passkey; the stream joins it. */
export function JoinWithPasskey({ token, defaultName }: { token: string; defaultName: string }) {
  const supported = useWebAuthnSupport();
  const signUp = usePasskeySignUp({ joinToken: token });
  const signIn = usePasskeySignIn({ joinToken: token });
  const nameRef = useRef<HTMLInputElement>(null);

  if (!supported) {
    return (
      <section className={authCard}>
        <AuthAlert tone="info">This browser doesn&apos;t support passkeys. Open this link in a current Chrome, Safari, Edge or Firefox.</AuthAlert>
      </section>
    );
  }

  const signUpError = signUp.error === ALREADY_HAS_PASSKEY ? "This device already has a hushush passkey. Sign in with it below instead." : signUp.error;

  return (
    <section className={authCard}>
      <div className="flex flex-col gap-2 sm:gap-1.5">
        <UnderlineField id="join-name" label="Your name" ref={nameRef} required maxLength={100} autoComplete="name" defaultValue={defaultName} />
        <p className={hintClass}>Shown to the company paying you.</p>
      </div>
      <div className="flex flex-col gap-2.5 sm:gap-2">
        {signUpError && <AuthAlert tone="bad">{signUpError}</AuthAlert>}
        <PasskeyButton
          status={signUp.status}
          label="Sign up and join"
          done="Joined"
          onClick={() => {
            const el = nameRef.current;
            if (el?.reportValidity()) void signUp.start(el.value.trim());
          }}
        />
        <p className={hintClass}>Creates a passkey with Face ID, your fingerprint, Windows Hello or device PIN. No password.</p>
      </div>
      <OrDivider />
      <div className="flex flex-col gap-2.5 sm:gap-2">
        <p className="m-0 text-[15px] font-semi">Already have a hushush account?</p>
        {signIn.error && <AuthAlert tone="bad">{signIn.error}</AuthAlert>}
        <PasskeyButton status={signIn.status} label="Sign in and join" done="Joined" onClick={() => void signIn.start()} secondary />
      </div>
    </section>
  );
}

/** Signed in: accept the stream on this account. */
export function AcceptStream({ token, name }: { token: string; name: string }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  return (
    <section className={authCard}>
      <p className="m-0 text-[15px] leading-[1.45] text-bark">
        Signed in as <b className="font-semi text-espresso">{name}</b>. The stream will be paid to this account.
      </p>
      {error && <AuthAlert tone="bad">{error}</AuthAlert>}
      <button
        type="button"
        className={primaryButton}
        disabled={pending}
        onClick={async () => {
          setPending(true);
          setError(undefined);
          const r = await api<{ streamId: string }>(`/api/join/${token}`);
          if (!r.ok || !r.data) {
            setPending(false);
            return setError(r.error);
          }
          router.push(`/app/streams/${r.data.streamId}`);
          router.refresh();
        }}
      >
        {pending ? "Joining…" : "Accept this stream"}
      </button>
    </section>
  );
}
