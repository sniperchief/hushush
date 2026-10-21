import { redirect } from "next/navigation";
import { PageHeader, SignInEmail } from "@/components/dashboard/sections";
import { accountEmail } from "@/lib/email-login";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "Sign-in email" };
export const dynamic = "force-dynamic";

export default async function AccountPage() {
  const user = await requirePageUser("/app/account");
  if (!user.intent) redirect("/app");
  const email = await accountEmail(user.id);
  return (
    <>
      <PageHeader title="Sign-in email" lead="A backup way into your account: a sign-in link sent to this address." />
      <div className="max-w-2xl">
        <SignInEmail email={email} />
      </div>
    </>
  );
}
