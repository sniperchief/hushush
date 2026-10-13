import Link from "next/link";
import { AutoRefresh } from "@/components/forms/stream-forms";
import { StreamTable } from "@/components/stream-table";
import { Badge, Card, CardTitle, Empty } from "@/components/ui";
import { formatClock, roleLabel } from "@/lib/format";
import { payroll } from "@/lib/payroll";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "Dashboard" };
export const dynamic = "force-dynamic";

export default async function Dashboard() {
  const user = await requirePageUser("/app");
  const [orgs, streams] = await Promise.all([payroll.listOrgs(user.id), payroll.myStreams(user.id)]);
  const asOf = streams[0]?.balance.as_of;

  return (
    <div className="space-y-6">
      <AutoRefresh />
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Welcome, {user.displayName}</h1>
        <p className="mt-1 text-sm text-ink-soft">Your organizations and the payroll streams paying you.</p>
      </div>

      <Card>
        <CardTitle
          action={
            <Link href="/app/orgs/new" className="inline-flex min-h-10 items-center rounded-xl bg-ink px-4 text-sm font-medium text-surface hover:opacity-90">
              New organization
            </Link>
          }
        >
          Organizations
        </CardTitle>
        {orgs.length === 0 ? (
          <Empty>You&apos;re not in any organization yet. Create one, or accept an invitation from an employer.</Empty>
        ) : (
          <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {orgs.map((o) => (
              <li key={o.organization_id}>
                <Link href={`/app/orgs/${o.organization_id}`} className="flex items-center justify-between gap-3 rounded-xl border border-line p-4 hover:bg-muted">
                  <span className="truncate font-medium">{o.organization_name}</span>
                  <Badge tone={o.role === "member" ? "neutral" : "accent"}>{roleLabel(o.role)}</Badge>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card>
        <CardTitle action={asOf ? <span className="text-xs text-ink-soft">Balances as of {formatClock(asOf)}</span> : undefined}>
          My payroll streams
        </CardTitle>
        {streams.length === 0 ? (
          <Empty>No one is paying you through hushush yet.</Empty>
        ) : (
          <StreamTable streams={streams} showOrg />
        )}
      </Card>
    </div>
  );
}
