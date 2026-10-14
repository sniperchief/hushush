import Link from "next/link";
import { redirect } from "next/navigation";
import { PasskeySignUp } from "@/components/passkey";
import { Card } from "@/components/ui";
import { currentUser } from "@/lib/session";

export const metadata = { title: "Employer sign-up" };

export default async function SignupPage() {
  if (await currentUser()) redirect("/app");
  return (
    <div className="mx-auto max-w-md space-y-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Create an employer account</h1>
        <p className="mt-1 text-sm text-ink-soft">Step 1 of 2: your passkey. Next you&apos;ll name your company.</p>
      </div>
      <Card>
        <PasskeySignUp kind="employer" />
      </Card>
      <p className="text-center text-sm text-ink-soft">
        Already have an account?{" "}
        <Link href="/login" className="font-medium text-ink underline">
          Sign in
        </Link>
        <br />
        Being paid? Use the worker link your employer sent you.
      </p>
    </div>
  );
}
