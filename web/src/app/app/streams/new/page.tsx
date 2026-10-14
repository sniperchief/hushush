import Link from "next/link";
import { redirect } from "next/navigation";
import { NewStreamForm } from "@/components/employer";
import { Card } from "@/components/ui";
import { employerCompany } from "@/lib/company";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "New stream" };
export const dynamic = "force-dynamic";

export default async function NewStreamPage() {
  const user = await requirePageUser("/app/streams/new");
  if (user.kind !== "employer" || !(await employerCompany(user))) redirect("/app");
  return (
    <div className="mx-auto max-w-2xl space-y-6">
      <div>
        <Link href="/app" className="text-sm text-ink-soft hover:underline">
          ← Home
        </Link>
        <h1 className="mt-2 text-2xl font-semibold tracking-tight">New stream</h1>
        <p className="mt-1 text-sm text-ink-soft">
          The worker earns the total evenly, by the second, from start to end. They can withdraw at each 20% milestone, and the rest once it ends.
        </p>
      </div>
      <Card>
        <NewStreamForm />
      </Card>
    </div>
  );
}
