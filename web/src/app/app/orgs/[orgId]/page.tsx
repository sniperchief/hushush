import Link from "next/link";
import { notFound } from "next/navigation";
import { InviteForm, MemberControls, RevokeInviteButton } from "@/components/forms/org-forms";
import { AutoRefresh, CreateStreamForm } from "@/components/forms/stream-forms";
import { StreamTable } from "@/components/stream-table";
import { Badge, Card, CardTitle, Empty } from "@/components/ui";
import { compactZec, formatClock, formatTime, roleLabel } from "@/lib/format";
import { listInvitations } from "@/lib/invitations";
import { payroll, PayrollApiError, type PayoutWalletSummary } from "@/lib/payroll";
import { requirePageUser } from "@/lib/session";

export const dynamic = "force-dynamic";

type Props = { params: Promise<{ orgId: string }> };

export default async function OrgPage({ params }: Props) {
  const { orgId } = await params;
  const user = await requirePageUser(`/app/orgs/${orgId}`);
  const membership = (await payroll.listOrgs(user.id)).find((o) => o.organization_id === orgId);
  if (!membership) notFound();
  const isManager = membership.role !== "member";

  if (!isManager) {
    const mine = (await payroll.myStreams(user.id)).filter((s) => s.organization_id === orgId);
    return (
      <div className="space-y-6">
        <AutoRefresh />
        <OrgHeader name={membership.organization_name} role={membership.role} />
        <Card>
          <CardTitle>Your streams in this organization</CardTitle>
          {mine.length === 0 ? <Empty>No streams yet. Your employer will set one up.</Empty> : <StreamTable streams={mine} showOrg />}
        </Card>
      </div>
    );
  }

  const [streams, members, invitations, wallet] = await Promise.all([
    payroll.listOrgStreams(user.id, orgId),
    payroll.listMembers(user.id, orgId),
    listInvitations(user.id, orgId),
    payoutWallet(user.id, orgId),
  ]);
  const asOf = streams[0]?.balance.as_of;

  return (
    <div className="space-y-6">
      <AutoRefresh />
      <OrgHeader name={membership.organization_name} role={membership.role} />

      <Card>
        <CardTitle action={asOf ? <span className="text-xs text-ink-soft">Balances as of {formatClock(asOf)}</span> : undefined}>
          Payroll streams
        </CardTitle>
        {streams.length === 0 ? <Empty>No streams yet. Create one below.</Empty> : <StreamTable streams={streams} showRecipient />}
      </Card>

      <Card>
        <CardTitle>New stream</CardTitle>
        <CreateStreamForm
          orgId={orgId}
          members={members.map((m) => ({ id: m.user_id, label: `${m.display_name} (${m.email})` }))}
        />
      </Card>

      {wallet !== "disabled" && <PayoutWalletCard wallet={wallet} />}

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardTitle>Members</CardTitle>
          <ul className="divide-y divide-line">
            {members.map((m) => (
              <li key={m.user_id} className="flex flex-wrap items-center justify-between gap-3 py-3">
                <div className="min-w-0">
                  <div className="truncate font-medium">
                    {m.display_name} {m.user_id === user.id && <span className="text-xs text-ink-soft">(you)</span>}
                  </div>
                  <div className="truncate text-xs text-ink-soft">{m.email}</div>
                </div>
                <div className="flex items-center gap-2">
                  {membership.role !== "owner" && <Badge tone={m.role === "member" ? "neutral" : "accent"}>{roleLabel(m.role)}</Badge>}
                  <MemberControls orgId={orgId} userId={m.user_id} role={m.role} actorRole={membership.role} isSelf={m.user_id === user.id} />
                </div>
              </li>
            ))}
          </ul>
        </Card>

        <Card>
          <CardTitle>Invite people</CardTitle>
          <InviteForm orgId={orgId} canInviteAdmins={membership.role === "owner"} />
          <h3 className="mb-2 mt-6 text-sm font-medium">Pending invitations</h3>
          {invitations.length === 0 ? (
            <p className="text-sm text-ink-soft">None.</p>
          ) : (
            <ul className="divide-y divide-line">
              {invitations.map((i) => (
                <li key={i.id} className="flex items-center justify-between gap-3 py-2.5 text-sm">
                  <div className="min-w-0">
                    <div className="truncate">{i.email}</div>
                    <div className="text-xs text-ink-soft">{roleLabel(i.role)} · expires {formatTime(Math.floor(Date.parse(i.expiresAt) / 1000))}</div>
                  </div>
                  <RevokeInviteButton orgId={orgId} invitationId={i.id} />
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>
    </div>
  );
}

/** The payout wallet for managers; "disabled" hides the card, "unavailable" shows a notice. */
async function payoutWallet(actor: string, orgId: string): Promise<PayoutWalletSummary | "disabled" | "unavailable"> {
  try {
    return await payroll.payoutWallet(actor, orgId);
  } catch (e) {
    if (e instanceof PayrollApiError && (e.code === "payouts_disabled" || e.status === 403 || e.status === 404)) return "disabled";
    return "unavailable";
  }
}

function PayoutWalletCard({ wallet }: { wallet: PayoutWalletSummary | "unavailable" }) {
  return (
    <Card>
      <CardTitle action={<Badge tone="amber">Testnet</Badge>}>Payout wallet</CardTitle>
      {wallet === "unavailable" ? (
        <p className="text-sm text-ink-soft">The payout wallet isn&apos;t reachable right now. Contributors can&apos;t be paid out until it is.</p>
      ) : (
        <div className="space-y-3 text-sm">
          <div className="flex flex-wrap gap-x-8 gap-y-2">
            <div>
              <div className="text-xs uppercase tracking-wide text-ink-soft">Spendable</div>
              <div className="font-mono text-lg font-semibold">{compactZec(wallet.spendable_zec)} TAZ</div>
            </div>
            {wallet.pending_zats > 0 && (
              <div>
                <div className="text-xs uppercase tracking-wide text-ink-soft">Awaiting confirmations</div>
                <div className="font-mono text-lg">{compactZec(wallet.pending_zec)} TAZ</div>
              </div>
            )}
          </div>
          <div>
            <div className="text-xs uppercase tracking-wide text-ink-soft">Funding address (shielded)</div>
            <div className="mt-1 break-all rounded-lg bg-muted px-3 py-2 font-mono text-xs">{wallet.funding_address}</div>
          </div>
          <p className="text-xs text-ink-soft">
            Contributors are paid from this wallet; each payout also pays a small network fee. Fund it only with testnet ZEC, from a shielded
            wallet: funding from a transparent address or exchange is public at that step. Payouts count as paid after {wallet.required_confirmations} confirmations.
          </p>
        </div>
      )}
    </Card>
  );
}

function OrgHeader({ name, role }: { name: string; role: string }) {
  return (
    <div>
      <Link href="/app" className="text-sm text-ink-soft hover:underline">← Dashboard</Link>
      <div className="mt-2 flex flex-wrap items-center gap-3">
        <h1 className="text-2xl font-semibold tracking-tight">{name}</h1>
        <Badge tone={role === "member" ? "neutral" : "accent"}>{roleLabel(role)}</Badge>
      </div>
    </div>
  );
}
