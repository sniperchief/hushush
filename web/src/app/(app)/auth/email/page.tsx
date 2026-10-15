import Link from "next/link";
import { EmailLinkConfirm } from "@/components/email-login";
import { Card } from "@/components/ui";
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
    <div className="mx-auto max-w-md space-y-6">
      <Card>
        {link && token ? (
          <EmailLinkConfirm token={token} purpose={link.purpose} email={link.email} />
        ) : (
          <div className="space-y-2">
            <h1 className="text-2xl font-semibold tracking-tight">This link has expired</h1>
            <p className="text-sm text-ink-soft">Email links work once and expire after 15 minutes. Request a new one.</p>
          </div>
        )}
      </Card>
      {!link && (
        <p className="text-center text-sm text-ink-soft">
          <Link href="/login" className="font-medium text-ink underline">
            Back to sign in
          </Link>
        </p>
      )}
    </div>
  );
}
