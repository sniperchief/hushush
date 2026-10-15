import Link from "next/link";
import { redirect } from "next/navigation";
import { SignUpMethods } from "@/components/sign-in-methods";
import { Card } from "@/components/ui";
import { currentUser } from "@/lib/session";

export const metadata = { title: "Sign up" };

export default async function SignupPage() {
  if (await currentUser()) redirect("/app");
  return (
    <div className="mx-auto max-w-md space-y-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Create your account</h1>
        <p className="mt-1 text-sm text-ink-soft">For paying a team, getting paid, or both.</p>
      </div>
      <Card>
        <SignUpMethods />
      </Card>
      <p className="text-center text-sm text-ink-soft">
        Already have an account?{" "}
        <Link href="/login" className="font-medium text-ink underline">
          Sign in
        </Link>
      </p>
    </div>
  );
}
