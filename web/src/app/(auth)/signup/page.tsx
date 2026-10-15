import { redirect } from "next/navigation";
import Link from "next/link";
import { AuthHeader, AuthMain, AuthTitle, HeaderLink, TestnetStrip } from "@/components/auth-shell";
import { growLink } from "@/components/auth-ui";
import { SignUpPanel } from "@/components/sign-in-methods";
import { currentUser } from "@/lib/session";

export const metadata = { title: "Sign up" };

export default async function SignupPage() {
  if (await currentUser()) redirect("/app");
  return (
    <>
      <TestnetStrip />
      <AuthHeader aside={<HeaderLink href="/login">Sign in</HeaderLink>} />
      <AuthMain>
        <AuthTitle title="Create your account" lead="For paying a team, getting paid, or both." />
        <SignUpPanel />
        <p className="m-0 mt-auto text-center text-[15px] text-bark sm:mt-0 sm:text-base">
          Already have an account?{" "}
          <Link href="/login" className={growLink}>
            Sign in
          </Link>
        </p>
      </AuthMain>
    </>
  );
}
