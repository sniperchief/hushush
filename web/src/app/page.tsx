import Link from "next/link";
import { redirect } from "next/navigation";
import { currentUser } from "@/lib/session";

export default async function Home() {
  if (await currentUser()) redirect("/app");
  return (
    <div className="mx-auto max-w-3xl py-10 text-center sm:py-20">
      <p className="mb-4 inline-block rounded-full bg-accent-bg px-3 py-1 text-xs font-medium text-accent-ink">Shielded ZEC payroll</p>
      <h1 className="text-4xl font-semibold tracking-tight sm:text-5xl">Pay your team by the second, privately.</h1>
      <p className="mx-auto mt-5 max-w-xl text-lg text-ink-soft">
        Set a fixed ZEC rate, and wages accrue continuously on the server. Contributors claim what they&apos;ve
        earned whenever they like.
      </p>
      <div className="mt-8 flex flex-wrap justify-center gap-3">
        <Link href="/signup" className="rounded-xl bg-ink px-5 py-3 text-sm font-medium text-surface hover:opacity-90">Create an account</Link>
        <Link href="/login" className="rounded-xl border border-line bg-surface px-5 py-3 text-sm font-medium hover:bg-muted">Sign in</Link>
      </div>
      <p className="mt-10 text-xs text-ink-faint">Testnet preview. Claims are reservations only; payouts are not enabled yet.</p>
    </div>
  );
}
