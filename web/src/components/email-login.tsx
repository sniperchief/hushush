"use client";

import { useRouter } from "next/navigation";
import { useState, type ReactNode } from "react";
import { api } from "@/lib/client-api";
import { AuthAlert, primaryButton, textButton, UnderlineField } from "./auth-ui";

/** Shown after any link request; never says whether the address has an account. */
function Sent({ email, children, onBack }: { email: string; children: ReactNode; onBack: () => void }) {
  return (
    <div className="flex flex-col gap-3">
      <AuthAlert tone="ok">
        <b className="font-semi">Check your email.</b> {children} It goes to <b className="font-semi break-all">{email}</b> and expires in 15 minutes.
      </AuthAlert>
      <button type="button" onClick={onBack} className={`${textButton} self-start`}>
        Use a different email
      </button>
    </div>
  );
}

/** Sends an email-link request; remembers the address once sent. */
export function useLinkRequest() {
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

/** Employer home: add or change the e-mail used for sign-in links. */
export function BackupEmailForm({ current }: { current: string | null }) {
  const { error, pending, sentTo, send, reset } = useLinkRequest();
  if (sentTo) {
    return (
      <Sent email={sentTo} onBack={reset}>
        Open the confirmation link we sent to start using this address.
      </Sent>
    );
  }
  return (
    <form
      className="flex flex-col gap-3.5 sm:flex-row sm:items-end"
      onSubmit={(e) => {
        e.preventDefault();
        const email = String(new FormData(e.currentTarget).get("email") ?? "").trim();
        void send("/api/account/email", { email });
      }}
    >
      <div className="min-w-0 flex-1">
        <UnderlineField
          id="sign-in-email-input"
          label={current ? "Change sign-in email" : "Sign-in email"}
          name="email"
          type="email"
          required
          maxLength={254}
          autoComplete="email"
          placeholder={current ?? "you@company.com"}
        />
      </div>
      <button
        type="submit"
        disabled={pending}
        className="h-[38px] flex-none cursor-pointer rounded-lg border border-espresso bg-transparent px-3.5 text-sm font-semi whitespace-nowrap text-espresso hover:bg-paper disabled:cursor-wait disabled:opacity-60"
      >
        {pending ? "Sending…" : "Send confirmation link"}
      </button>
      {error && (
        <div className="sm:basis-full">
          <AuthAlert tone="bad">{error}</AuthAlert>
        </div>
      )}
    </form>
  );
}

const ACTION = {
  signup: { title: "Create your account", lead: "Continue as ", tail: ".", button: "Create account" },
  login: { title: "Sign in", lead: "Continue as ", tail: ".", button: "Sign in" },
  link: { title: "Confirm your sign-in email", lead: "Use ", tail: " for sign-in links.", button: "Confirm email" },
} as const;

/** The e-mailed link's landing page: one click uses the link (mail scanners only GET it). */
export function EmailLinkConfirm({ token, purpose, email }: { token: string; purpose: keyof typeof ACTION; email: string }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [status, setStatus] = useState<"idle" | "wait" | "opening" | "linked">("idle");
  const a = ACTION[purpose];

  return (
    <div className="flex flex-col gap-[22px]">
      <div className="flex flex-col gap-2">
        <h1 className="m-0 text-[32px] leading-[1.1] font-semi tracking-[-0.02em]">{a.title}</h1>
        <p className="m-0 text-base leading-[1.45] text-bark">
          {a.lead}
          <b className="font-semi break-all text-espresso">{email}</b>
          {a.tail}
        </p>
      </div>
      {status === "linked" && (
        <AuthAlert tone="ok">
          <b className="font-semi break-all">{email}</b> is now your sign-in email. You can use it to get a sign-in link if you ever lose your passkey.
        </AuthAlert>
      )}
      {error && <AuthAlert tone="bad">{error}</AuthAlert>}
      <button
        type="button"
        className={primaryButton}
        disabled={status === "wait" || status === "opening"}
        onClick={async () => {
          if (status === "linked") return router.push("/app");
          setStatus("wait");
          setError(undefined);
          const r = await api<{ purpose: string }>("/api/auth/email/verify", "POST", { token });
          if (!r.ok) {
            setStatus("idle");
            return setError(r.error);
          }
          if (purpose === "link") return setStatus("linked");
          setStatus("opening");
          router.replace("/app");
          router.refresh();
        }}
      >
        {status === "wait" ? "One moment…" : status === "linked" ? "Go to dashboard" : status === "opening" ? "Opening your dashboard…" : a.button}
      </button>
      <p className="m-0 text-[13px] leading-[1.4] text-bark">Opening this page doesn&apos;t use the link. Only the button does, so email scanners can&apos;t spend it.</p>
    </div>
  );
}
