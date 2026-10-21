import { redirect } from "next/navigation";
import { PageHeader } from "@/components/dashboard/sections";
import { NewStreamForm } from "@/components/employer";
import { Card } from "@/components/ui";
import { employerCompany } from "@/lib/company";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "New stream" };
export const dynamic = "force-dynamic";

export default async function NewStreamPage() {
  const user = await requirePageUser("/app/streams/new");
  if (!(await employerCompany(user))) redirect("/app");
  return (
    <div className="flex w-full max-w-2xl flex-col gap-4 min-[900px]:gap-5">
      <PageHeader
        back={{ href: "/app/streams", label: "Streams" }}
        title="New stream"
        lead="The worker earns the total evenly, by the second, from start to end. They can withdraw at each 20% milestone, and the rest once it ends."
      />
      <Card>
        <NewStreamForm />
      </Card>
    </div>
  );
}
