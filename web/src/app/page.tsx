import Link from "next/link";
import { redirect } from "next/navigation";
import { currentUser } from "@/lib/session";

export default async function Home() {
  if (await currentUser()) redirect("/app");
  return (
    <div className="mx-auto max-w-3xl py-10 text-center sm:py-20">
      <p className="mb-4 inline-block rounded-full bg-accent-bg px-3 py-1 text-xs font-medium text-accent-ink">Shielded ZEC payroll · testnet</p>
      <h1 className="text-4xl font-semibold tracking-tight sm:text-5xl">Pay people by the second, privately.</h1>
      <p className="mx-auto mt-5 max-w-xl text-lg text-ink-soft">
        Set a total and an end date. Earnings tick up every second, and your worker withdraws to their shielded Zcash address at each 20%
        milestone.
      </p>
      <div className="mt-8 flex flex-wrap justify-center gap-3">
        <Link href="/signup" className="rounded-xl bg-ink px-5 py-3 text-sm font-medium text-surface hover:opacity-90">
          I&apos;m an employer — get started
        </Link>
        <Link href="/login" className="rounded-xl border border-line bg-surface px-5 py-3 text-sm font-medium hover:bg-muted">
          Sign in
        </Link>
      </div>
      <p className="mx-auto mt-10 max-w-md text-xs text-ink-faint">Being paid? Open the worker link your employer sent you. Testnet only: TAZ has no value.</p>
    </div>
  );
}
