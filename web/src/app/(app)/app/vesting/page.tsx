import Link from "next/link";
import { redirect } from "next/navigation";
import { Avatar } from "@/components/dashboard/avatar";
import { ActionLink, Card, EmptyBox, MoreLink, PageHeader, VestingBar, VestingTag } from "@/components/dashboard/sections";
import { AutoRefresh } from "@/components/worker";
import type { SessionUser } from "@/lib/auth";
import { rangeLabel } from "@/lib/chart-data";
import type { Company } from "@/lib/company";
import { companyOf, myVestingOf, orgVestingOf } from "@/lib/dashboard";
import { compactZec, formatTime } from "@/lib/format";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "Vesting" };
export const dynamic = "force-dynamic";

export default async function VestingListPage() {
  const user = await requirePageUser("/app/vesting");
  if (!user.intent) redirect("/app");
  if (user.intent === "pay") {
    const company = await companyOf(user.id);
    if (!company) redirect("/app");
    return <EmployerVesting user={user} company={company} />;
  }
  return <EmployeeVesting user={user} />;
}

async function EmployerVesting({ user, company }: { user: SessionUser; company: Company }) {
  const vesting = await orgVestingOf(user.id, company.id);
  return (
    <>
      <AutoRefresh seconds={30} />
      <PageHeader
        title="Vesting"
        lead="Fixed ZEC allocations that vest to one person over time, with an optional cliff. They withdraw what has vested, paid from your payout wallet."
        action={<ActionLink href="/app/vesting/new">New agreement</ActionLink>}
      />
      <Card id="vesting" title="Agreements" icon="vesting" count={vesting.length}>
        {vesting.length === 0 ? (
          <EmptyBox>No vesting agreements. Grant someone in your company a ZEC allocation that vests over time, with an optional cliff.</EmptyBox>
        ) : (
          <ul className="m-0 flex list-none flex-col p-0">
            {vesting.map((v) => (
              <li key={v.id} className="flex flex-col gap-2 border-b border-mist py-3.5 first:pt-0 last:border-0 last:pb-0">
                <div className="flex items-center justify-between gap-2.5">
                  <Link href={`/app/vesting/${v.id}`} className="flex min-w-0 items-center gap-2.5 font-semi text-espresso no-underline hover:underline">
                    <Avatar name={v.employee_display_name} />
                    <span className="truncate">{v.employee_display_name}</span>
                  </Link>
                  <VestingTag v={v} />
                </div>
                <VestingBar v={v} />
                <div className="flex flex-wrap justify-between gap-x-4 gap-y-1 text-[12.5px] text-bark">
                  <span>
                    Vested <span className="mono text-espresso">{compactZec(v.balance.vested_zec)}</span> of <span className="mono">{compactZec(v.total_zec)}</span> ZEC
                  </span>
                  <span>
                    Paid <span className="mono text-espresso">{compactZec(v.balance.paid_zec)}</span>
                    {v.balance.reserved_zats > 0 && (
                      <>
                        {" "}
                        · on its way <span className="mono">{compactZec(v.balance.reserved_zec)}</span>
                      </>
                    )}
                  </span>
                  <span>
                    {v.status === "draft"
                      ? v.activatable
                        ? `Activate by ${formatTime(v.start_at)}`
                        : "Start passed: discard and recreate"
                      : `Ends ${formatTime(v.end_at)}`}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}

async function EmployeeVesting({ user }: { user: SessionUser }) {
  const vesting = await myVestingOf(user.id);
  return (
    <>
      <AutoRefresh seconds={15} />
      <PageHeader title="Vesting" lead="ZEC allocations vesting to you over time. Withdraw what has vested to your locked payout address." />
      <Card id="vesting" title="Agreements" icon="vesting" count={vesting.length}>
        {vesting.length === 0 ? (
          <EmptyBox>No vesting agreements yet. When a company grants you one, it appears here once it&apos;s active.</EmptyBox>
        ) : (
          <ul className="m-0 flex list-none flex-col p-0">
            {vesting.map((v) => (
              <li key={v.id} className="flex flex-col gap-2 border-b border-mist py-3.5 first:pt-0 last:border-0 last:pb-0">
                <div className="flex items-center justify-between gap-2.5">
                  <Link href={`/app/vesting/${v.id}`} className="min-w-0 truncate font-semi text-espresso no-underline hover:underline">
                    {v.organization_name} · <span className="mono font-medium">{compactZec(v.total_zec)}</span> ZEC
                  </Link>
                  <VestingTag v={v} />
                </div>
                <VestingBar v={v} />
                <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 text-[12.5px] text-bark">
                  <span>
                    Vested <span className="mono text-espresso">{compactZec(v.balance.vested_zec)}</span> · paid{" "}
                    <span className="mono text-espresso">{compactZec(v.balance.paid_zec)}</span>
                    {v.balance.reserved_zats > 0 && (
                      <>
                        {" "}
                        · on its way <span className="mono">{compactZec(v.balance.reserved_zec)}</span>
                      </>
                    )}
                  </span>
                  <span>
                    Available <span className="mono font-semi text-espresso">{compactZec(v.balance.available_zec)}</span> ZEC
                  </span>
                </div>
                <div className="flex flex-wrap items-center justify-between gap-2 text-[12.5px] text-bark">
                  <span>{rangeLabel(v.start_at, v.end_at)}</span>
                  <MoreLink href={`/app/vesting/${v.id}`}>{v.balance.available_zats > 0 ? "Withdraw →" : "Details →"}</MoreLink>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}
