import Link from "next/link";
import { redirect } from "next/navigation";
import { LoginForm } from "@/components/forms/auth-forms";
import { Card } from "@/components/ui";
import { currentUser, safeNext } from "@/lib/session";

export const metadata = { title: "Sign in" };

type Props = { searchParams: Promise<Record<string, string | string[] | undefined>> };

export default async function LoginPage({ searchParams }: Props) {
  const sp = await searchParams;
  const next = safeNext(sp.next);
  if (await currentUser()) redirect(next);
  return (
    <div className="mx-auto max-w-md">
      <h1 className="mb-6 text-2xl font-semibold tracking-tight">Sign in</h1>
      <Card>
        <LoginForm next={next} />
      </Card>
      <p className="mt-4 text-center text-sm text-ink-soft">
        New here? <Link href={`/signup?next=${encodeURIComponent(next)}`} className="font-medium text-ink underline">Create an account</Link>
      </p>
    </div>
  );
}
