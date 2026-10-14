import Link from "next/link";
import { redirect } from "next/navigation";
import type { ReactNode } from "react";
import { LogoutButton } from "@/components/forms/logout-button";
import { JoinButton } from "@/components/join-button";
import { PasskeySignIn, PasskeySignUp } from "@/components/passkey";
import { Alert, Card } from "@/components/ui";
import { linkHashOf } from "@/lib/company";
import { compactZec, formatTime } from "@/lib/format";
import { PayrollApiError, payroll, type LinkPreview } from "@/lib/payroll";
import { currentUser } from "@/lib/session";

export const metadata = { title: "Your stream" };
export const dynamic = "force-dynamic";

type Props = { params: Promise<{ token: string }> };

function Unavailable({ children }: { children: ReactNode }) {
  return (
    <div className="mx-auto max-w-md space-y-4 py-10 text-center">
      <h1 className="text-2xl font-semibold tracking-tight">Link unavailable</h1>
      <p className="text-sm text-ink-soft">{children}</p>
      <Link href="/" className="inline-flex min-h-10 items-center rounded-xl bg-ink px-4 text-sm font-medium text-surface">
        hushush home
      </Link>
    </div>
  );
}

/**
 * The worker link. Before sign-in it shows only the terms (company, total,
 * dates); the worker then signs up or signs in with a passkey and the stream
 * binds to their account. Employer accounts are told to send it on instead.
 */
export default async function JoinPage({ params }: Props) {
  const { token } = await params;
  const hash = linkHashOf(token);
  let preview: LinkPreview | null = null;
  if (hash) {
    try {
      preview = await payroll.linkPreview(hash);
    } catch (e) {
      if (!(e instanceof PayrollApiError && e.status === 404)) throw e;
    }
  }
  if (!hash || !preview) return <Unavailable>This worker link is not valid. Ask your employer for a new one.</Unavailable>;
  if (preview.status === "cancelled") return <Unavailable>{preview.organization_name} cancelled this stream.</Unavailable>;

  const user = await currentUser();
  if (preview.status === "bound") {
    // Binding is idempotent for the worker who owns it (it returns their
    // stream) and refused for anyone else, so this reveals nothing.
    if (user?.kind === "worker") {
      let streamId: string | null = null;
      try {
        streamId = (await payroll.bindLink(user.id, hash)).stream_id;
      } catch (e) {
        if (!(e instanceof PayrollApiError && e.status < 500)) throw e;
      }
      if (streamId) redirect(`/app/streams/${streamId}`);
    }
    return <Unavailable>This worker link has already been used. Each link works once.</Unavailable>;
  }

  return (
    <div className="mx-auto max-w-md space-y-6">
      <Card>
        <p className="text-xs font-medium uppercase tracking-wide text-ink-soft">Hi {preview.worker_name}</p>
        <h1 className="mt-2 text-xl font-semibold tracking-tight">
          {preview.organization_name} is paying you {compactZec(preview.total_zec)} ZEC
        </h1>
        <p className="mt-1 text-sm text-ink-soft">
          from {formatTime(preview.start_at)} to {formatTime(preview.end_at)}, accruing every second. You can withdraw at each 20% milestone, and
          whatever is left once it ends.
        </p>
        <p className="mt-3 text-xs text-ink-faint">Zcash testnet: TAZ has no monetary value.</p>
      </Card>

      {user?.kind === "employer" ? (
        <Card>
          <Alert tone="amber">
            You&apos;re signed in as an employer ({user.displayName}). This link is for the worker being paid — send it to them. To accept it
            yourself with a worker account, sign out first.
          </Alert>
          <div className="mt-4 flex flex-wrap gap-2">
            <Link href="/app" className="inline-flex min-h-10 items-center rounded-xl bg-ink px-4 text-sm font-medium text-surface">
              Back to my company
            </Link>
            <LogoutButton redirectTo={`/join/${token}`} />
          </div>
        </Card>
      ) : user?.kind === "worker" ? (
        <Card>
          <p className="mb-4 text-sm text-ink-soft">Signed in as {user.displayName}.</p>
          <JoinButton token={token} />
        </Card>
      ) : (
        <>
          <Card>
            <h2 className="mb-4 text-base font-semibold tracking-tight">New to hushush? Sign up</h2>
            <PasskeySignUp kind="worker" defaultName={preview.worker_name} joinToken={token} />
          </Card>
          <Card>
            <h2 className="mb-3 text-base font-semibold tracking-tight">Already have a worker account?</h2>
            <PasskeySignIn joinToken={token} variant="secondary" />
          </Card>
        </>
      )}
    </div>
  );
}
