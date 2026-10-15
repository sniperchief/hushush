import Link from "next/link";
import { redirect } from "next/navigation";
import { SignInMethods } from "@/components/sign-in-methods";
import { Card } from "@/components/ui";
import { currentUser, safeNext } from "@/lib/session";

export const metadata = { title: "Sign in" };

type Props = { searchParams: Promise<{ next?: string | string[] }> };

export default async function LoginPage({ searchParams }: Props) {
  const next = safeNext((await searchParams).next);
  if (await currentUser()) redirect(next);
  return (
    <div className="mx-auto max-w-md space-y-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Sign in</h1>
        <p className="mt-1 text-sm text-ink-soft">Use your passkey (Face ID, fingerprint, Windows Hello or device PIN), or get a sign-in link by email.</p>
      </div>
      <Card>
        <SignInMethods next={next} />
      </Card>
      <p className="text-center text-sm text-ink-soft">
        New to hushush?{" "}
        <Link href="/signup" className="font-medium text-ink underline">
          Create an account
        </Link>
      </p>
    </div>
  );
}
