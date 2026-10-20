import Link from "next/link";
import { redirect } from "next/navigation";
import { Card } from "@/components/ui";
import { NewVestingForm } from "@/components/vesting";
import { employerCompany } from "@/lib/company";
import { payroll } from "@/lib/payroll";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "New vesting agreement" };
export const dynamic = "force-dynamic";

export default async function NewVestingPage() {
  const user = await requirePageUser("/app/vesting/new");
  const company = await employerCompany(user);
  if (!company) redirect("/app");
  // Eligible: the company's members other than the employer (payroll checks it again).
  const members = await payroll.listMembers(user.id, company.id);
  const employees = members.filter((m) => m.user_id !== user.id).map((m) => ({ id: m.user_id, name: m.display_name }));
  return (
    <div className="mx-auto max-w-2xl space-y-6">
      <div>
        <Link href="/app" className="text-sm text-ink-soft hover:underline">
          ← Home
        </Link>
        <h1 className="mt-2 text-2xl font-semibold tracking-tight">New vesting agreement</h1>
        <p className="mt-1 text-sm text-ink-soft">
          A fixed ZEC allocation that vests to one person over time, with an optional cliff. They withdraw what has vested, paid from your
          company&apos;s payout wallet. Streams remain the way to pay for ongoing work.
        </p>
      </div>
      <Card>
        <NewVestingForm employees={employees} />
      </Card>
    </div>
  );
}
