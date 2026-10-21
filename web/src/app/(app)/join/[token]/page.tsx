import Link from "next/link";
import { redirect } from "next/navigation";
import type { ReactNode } from "react";
import { AuthHeader, AuthMain, HeaderLink, TestnetStrip } from "@/components/auth-shell";
import { primaryButton, secondaryButton } from "@/components/auth-ui";
import { AcceptStream, JoinWithPasskey } from "@/components/join-panel";
import { initials } from "@/lib/chart-data";
import { linkHashOf } from "@/lib/company";
import { compactZec, formatTime } from "@/lib/format";
import { PayrollApiError, payroll, type LinkPreview } from "@/lib/payroll";
import { currentUser } from "@/lib/session";

export const metadata = { title: "Your stream" };
export const dynamic = "force-dynamic";

type Props = { params: Promise<{ token: string }> };

function Frame({ aside, children }: { aside?: ReactNode; children: ReactNode }) {
  return (
    <>
      <TestnetStrip />
      <AuthHeader aside={aside} asideAlways />
      <AuthMain>{children}</AuthMain>
    </>
  );
}

/** `signInTo`: offer to sign in and come back here (the worker who used this link). */
function Unavailable({ children, signInTo }: { children: ReactNode; signInTo?: string }) {
  return (
    <Frame>
      <div className="flex flex-col gap-2 sm:gap-1.5 sm:pt-10 sm:text-center">
        <h1 className="m-0 text-[32px] leading-[1.08] font-semi tracking-[-0.8px] sm:text-[28px] sm:leading-[1.1]">Link unavailable</h1>
        <p className="m-0 text-base leading-[1.45] text-bark sm:text-[15px]">{children}</p>
      </div>
      {signInTo ? (
        <div className="flex flex-col gap-3">
          <p className="m-0 text-[15px] leading-[1.45] sm:text-center">
            <b className="font-semi">Already joined with this link?</b> Sign in to open your stream.
          </p>
          <Link href={`/login?next=${encodeURIComponent(signInTo)}`} className={primaryButton}>
            Sign in
          </Link>
          <Link href="/" className={secondaryButton}>
            Go to hushush
          </Link>
        </div>
      ) : (
        <Link href="/" className={primaryButton}>
          Go to hushush
        </Link>
      )}
    </Frame>
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
    if (user) {
      let streamId: string | null = null;
      try {
        streamId = (await payroll.bindLink(user.id, hash)).stream_id;
      } catch (e) {
        if (!(e instanceof PayrollApiError && e.status < 500)) throw e;
      }
      if (streamId) redirect(`/app/streams/${streamId}`);
    }
    // Signed out, this may be the worker who joined, in another browser:
    // signing in brings them back here, and the link then opens their stream.
    return (
      <Unavailable signInTo={user ? undefined : `/join/${token}`}>This worker link has already been used. Each link works once.</Unavailable>
    );
  }

  return (
    <Frame aside={user ? <span className="flex min-h-11 items-center px-3 text-sm text-bark">{user.displayName}</span> : <HeaderLink href="/login">Sign in</HeaderLink>}>
      <div className="flex flex-col gap-2 sm:gap-1.5 sm:text-center">
        <p className="m-0 text-[13px] font-semi tracking-[.08em] text-bark uppercase">Hi {preview.worker_name}</p>
        <h1 className="m-0 text-[32px] leading-[1.08] font-semi tracking-[-0.8px] text-balance sm:text-[28px] sm:leading-[1.12] sm:tracking-[-0.025em]">
          {preview.organization_name} is paying you {compactZec(preview.total_zec)} ZEC
        </h1>
        <p className="m-0 text-base leading-[1.45] text-bark sm:text-[15px]">A private stream of pay, earned every second and paid in shielded ZEC.</p>
      </div>

      <section aria-label="Stream terms" className="flex flex-col gap-3.5 rounded-xl border border-linen bg-paper p-5 shadow-card">
        <div className="flex items-center gap-3">
          <span aria-hidden="true" className="flex size-10 flex-none items-center justify-center rounded-[9px] bg-espresso text-sm font-semi text-cream">
            {initials(preview.organization_name)}
          </span>
          <div className="min-w-0">
            <div className="truncate text-[15px] font-semi">{preview.organization_name}</div>
            <div className="mono text-[13px] text-bark">{compactZec(preview.total_zec)} ZEC in total</div>
          </div>
        </div>
        <dl className="m-0 grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 border-t border-mist pt-3.5 text-sm">
          <dt className="text-bark">Starts</dt>
          <dd className="m-0 text-right">{formatTime(preview.start_at)}</dd>
          <dt className="text-bark">Ends</dt>
          <dd className="m-0 text-right">{formatTime(preview.end_at)}</dd>
        </dl>
        <p className="m-0 border-t border-mist pt-3.5 text-[13.5px] leading-[1.45] text-bark">
          You earn it evenly, every second. Withdraw at each 20% milestone, and whatever is left once it ends, to a shielded address you choose.
        </p>
      </section>

      {user ? <AcceptStream token={token} name={user.displayName} /> : <JoinWithPasskey token={token} defaultName={preview.worker_name} />}

      <p className="m-0 text-center text-[12.5px] text-bark">Zcash testnet: TAZ has no monetary value.</p>
    </Frame>
  );
}
