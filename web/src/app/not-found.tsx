import Link from "next/link";

export default function NotFound() {
  return (
    <main className="mx-auto max-w-md px-4 py-24 text-center">
      <h1 className="text-2xl font-semibold tracking-tight">Not found</h1>
      <p className="mt-2 text-sm text-ink-soft">This page doesn&apos;t exist, or you don&apos;t have access to it.</p>
      <Link href="/app" className="mt-6 inline-flex min-h-10 items-center rounded-xl bg-ink px-4 text-sm font-medium text-surface">Go to dashboard</Link>
    </main>
  );
}
