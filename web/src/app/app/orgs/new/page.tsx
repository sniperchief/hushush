import { CreateOrgForm } from "@/components/forms/org-forms";
import { Card } from "@/components/ui";

export const metadata = { title: "New organization" };

export default function NewOrgPage() {
  return (
    <div className="mx-auto max-w-lg">
      <h1 className="mb-6 text-2xl font-semibold tracking-tight">New organization</h1>
      <Card>
        <p className="mb-4 text-sm text-ink-soft">You&apos;ll be the owner. You can invite admins and contributors next.</p>
        <CreateOrgForm />
      </Card>
    </div>
  );
}
