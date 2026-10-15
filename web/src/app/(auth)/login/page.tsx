import { redirect } from "next/navigation";
import Link from "next/link";
import { AuthHeader, AuthMain, AuthTitle, HeaderLink, TestnetStrip } from "@/components/auth-shell";
import { growLink } from "@/components/auth-ui";
import { SignInPanel } from "@/components/sign-in-methods";
import { currentUser, safeNext } from "@/lib/session";

export const metadata = { title: "Sign in" };

type Props = { searchParams: Promise<{ next?: string | string[] }> };

export default async function LoginPage({ searchParams }: Props) {
  const next = safeNext((await searchParams).next);
  if (await currentUser()) redirect(next);
  return (
    <>
      <TestnetStrip />
      <AuthHeader aside={<HeaderLink href="/signup">Sign up</HeaderLink>} />
      <AuthMain>
        <AuthTitle title="Sign in" lead="Use your passkey, or get a sign-in link by email." />
        <SignInPanel next={next} />
        <p className="m-0 mt-auto text-center text-[15px] text-bark sm:mt-0 sm:text-base">
          New to hushush?{" "}
          <Link href="/signup" className={growLink}>
            Create an account
          </Link>
        </p>
      </AuthMain>
    </>
  );
}
