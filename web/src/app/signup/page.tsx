import Link from "next/link";
import { redirect } from "next/navigation";
import { SignupForm } from "@/components/forms/auth-forms";
import { Card } from "@/components/ui";
import { currentUser, safeNext } from "@/lib/session";

export const metadata = { title: "Create account" };

type Props = { searchParams: Promise<Record<string, string | string[] | undefined>> };

export default async function SignupPage({ searchParams }: Props) {
  const sp = await searchParams;
  const next = safeNext(sp.next);
  if (await currentUser()) redirect(next);
  const email = typeof sp.email === "string" ? sp.email : undefined;
  return (
    <div className="mx-auto max-w-md">
      <h1 className="mb-6 text-2xl font-semibold tracking-tight">Create your account</h1>
      <Card>
        <SignupForm next={next} email={email} />
      </Card>
      <p className="mt-4 text-center text-sm text-ink-soft">
        Have an account? <Link href={`/login?next=${encodeURIComponent(next)}`} className="font-medium text-ink underline">Sign in</Link>
      </p>
    </div>
  );
}
