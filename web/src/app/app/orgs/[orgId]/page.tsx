import Link from "next/link";
import { notFound } from "next/navigation";
import { InviteForm, MemberControls, RevokeInviteButton } from "@/components/forms/org-forms";
import { AutoRefresh, CreateStreamForm } from "@/components/forms/stream-forms";
import { StreamTable } from "@/components/stream-table";
import { Badge, Card, CardTitle, Empty } from "@/components/ui";
import { formatClock, formatTime, roleLabel } from "@/lib/format";
import { listInvitations } from "@/lib/invitations";
import { payroll } from "@/lib/payroll";
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

  const [streams, members, invitations] = await Promise.all([
    payroll.listOrgStreams(user.id, orgId),
    payroll.listMembers(user.id, orgId),
    listInvitations(user.id, orgId),
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
