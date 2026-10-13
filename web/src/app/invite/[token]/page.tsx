import Link from "next/link";
import { AcceptInviteButton } from "@/components/forms/org-forms";
import { Alert, Card } from "@/components/ui";
import { roleLabel } from "@/lib/format";
import { previewInvitation } from "@/lib/invitations";
import { currentUser } from "@/lib/session";

export const metadata = { title: "Invitation" };

type Props = { params: Promise<{ token: string }> };

export default async function InvitePage({ params }: Props) {
  const { token } = await params;
  const [inv, user] = await Promise.all([previewInvitation(token), currentUser()]);
  const here = `/invite/${encodeURIComponent(token)}`;

  return (
    <div className="mx-auto max-w-md">
      <h1 className="mb-6 text-2xl font-semibold tracking-tight">Join an organization</h1>
      <Card>
        {!inv ? (
          <Alert>This invitation link isn&apos;t valid.</Alert>
        ) : inv.state !== "open" ? (
          <Alert tone="amber">This invitation has been {inv.state === "expired" ? "expired" : inv.state}. Ask for a new one.</Alert>
        ) : (
          <div className="space-y-4">
            <p>
              <strong>{inv.inviterName}</strong> invited <strong>{inv.email}</strong> to join{" "}
              <strong>{inv.organizationName}</strong> as {inv.role === "admin" ? "an admin" : "a contributor"}.
            </p>
            <p className="text-sm text-ink-soft">Role: {roleLabel(inv.role)}</p>
            {!user ? (
              <div className="flex flex-wrap gap-2">
                <Link href={`/signup?next=${encodeURIComponent(here)}&email=${encodeURIComponent(inv.email)}`} className="inline-flex min-h-10 items-center rounded-xl bg-ink px-4 text-sm font-medium text-surface">
                  Create account
                </Link>
                <Link href={`/login?next=${encodeURIComponent(here)}`} className="inline-flex min-h-10 items-center rounded-xl border border-line px-4 text-sm font-medium">
                  Sign in
                </Link>
              </div>
            ) : user.email.toLowerCase() !== inv.email.toLowerCase() ? (
              <Alert tone="amber">
                You&apos;re signed in as {user.email}. This invitation is for {inv.email}. Sign out and sign in with that address to accept it.
              </Alert>
            ) : (
              <AcceptInviteButton token={token} />
            )}
          </div>
        )}
      </Card>
    </div>
  );
}
