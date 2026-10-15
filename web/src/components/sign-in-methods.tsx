"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { EmailSignIn, EmailSignUp } from "./email-login";
import { PasskeySignIn, PasskeySignUp, useWebAuthnSupport } from "./passkey";

type Method = "passkey" | "email";

/**
 * One sign-in method at a time: passkey first, email one click away. Browsers
 * without passkey support get the email form straight away.
 */
function Methods({ passkey, email, emailNote }: { passkey: ReactNode; email: ReactNode; emailNote?: ReactNode }) {
  const supported = useWebAuthnSupport();
  const [chosen, setChosen] = useState<Method>("passkey");
  const method: Method = supported ? chosen : "email";
  const switched = useRef(false);
  const box = useRef<HTMLDivElement>(null);

  // After a switch, put focus in the form that just appeared.
  useEffect(() => {
    if (!switched.current) return;
    box.current?.querySelector<HTMLElement>("input, button")?.focus();
  }, [method]);

  const switchTo = (m: Method) => {
    switched.current = true;
    setChosen(m);
  };

  return (
    <div className="space-y-4">
      <div ref={box}>{method === "passkey" ? passkey : email}</div>
      {method === "email" && emailNote}
      {supported && (
        <button
          type="button"
          onClick={() => switchTo(method === "passkey" ? "email" : "passkey")}
          className="block w-full text-center text-sm text-ink-soft underline underline-offset-2 hover:text-ink"
        >
          {method === "passkey" ? "Use email instead" : "Use a passkey instead"}
        </button>
      )}
    </div>
  );
}

export function SignInMethods({ next }: { next: string }) {
  return (
    <Methods
      passkey={<PasskeySignIn next={next} />}
      email={<EmailSignIn />}
      emailNote={<p className="text-xs text-ink-soft">Works if you signed up with email or added a sign-in email to your account.</p>}
    />
  );
}

/** Sign-up for everyone. The name typed in one form carries over to the other. */
export function SignUpMethods() {
  const [name, setName] = useState("");
  return (
    <Methods
      passkey={<PasskeySignUp defaultName={name} onNameChange={setName} />}
      email={<EmailSignUp defaultName={name} onNameChange={setName} />}
    />
  );
}
