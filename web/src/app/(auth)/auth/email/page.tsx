import Link from "next/link";
import { primaryButton } from "@/components/auth-ui";
import { EmailLinkConfirm } from "@/components/email-login";
import { Wordmark } from "@/components/landing/ui";
import { peekLink } from "@/lib/email-login";

export const metadata = { title: "Email link", robots: { index: false } };
export const dynamic = "force-dynamic";

type Props = { searchParams: Promise<{ token?: string | string[] }> };

/** Landing page for e-mailed links. Looking at it never uses the link; its button does. */
export default async function EmailLinkPage({ searchParams }: Props) {
  const raw = (await searchParams).token;
  const token = Array.isArray(raw) ? raw[0] : raw;
  const link = await peekLink(token);
  return (
    <div className="flex flex-1 flex-col items-center px-5 py-[clamp(24px,4vw,48px)]">
      <Link href="/" aria-label="hushush home" className="self-start">
        <Wordmark className="h-7" alt="" />
      </Link>
      <main className="flex w-full flex-1 items-center justify-center py-10">
        <div className="flex w-full max-w-[460px] flex-col gap-[22px] rounded-xl bg-paper p-[clamp(28px,5vw,44px)] shadow-float">
          {link && token ? (
            <EmailLinkConfirm token={token} purpose={link.purpose} email={link.email} />
          ) : (
            <div className="flex flex-col gap-3.5">
              <h1 className="m-0 text-[32px] leading-[1.1] font-semi tracking-[-0.02em]">This link has expired</h1>
              <p className="m-0 text-base leading-[1.45] text-bark">Email links work once and expire after 15 minutes. Request a new one.</p>
              <Link href="/login" className={`${primaryButton} mt-2`}>
                Back to sign in
              </Link>
            </div>
          )}
        </div>
      </main>
    </div>
  );
}
