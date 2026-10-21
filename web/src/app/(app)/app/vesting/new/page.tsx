import { redirect } from "next/navigation";
import { PageHeader } from "@/components/dashboard/sections";
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
    <div className="flex w-full max-w-2xl flex-col gap-4 min-[900px]:gap-5">
      <PageHeader
        back={{ href: "/app/vesting", label: "Vesting" }}
        title="New vesting agreement"
        lead={
          <>
            A fixed ZEC allocation that vests to one person over time, with an optional cliff. They withdraw what has vested, paid from your
            company&apos;s payout wallet. Streams remain the way to pay for ongoing work.
          </>
        }
      />
      <Card>
        <NewVestingForm employees={employees} />
      </Card>
    </div>
  );
}
