import Link from "next/link";
import { AuthHeader, TestnetStrip } from "@/components/auth-shell";

export default function NotFound() {
  return (
    <div className="landing flex min-h-dvh flex-col bg-cream font-landing text-espresso">
      <TestnetStrip />
      <AuthHeader />
      <main className="flex flex-1 justify-center px-5 pt-16 pb-10">
        <div className="flex w-full max-w-[420px] flex-col items-center gap-3 text-center">
          <p className="m-0 mono text-sm text-bark">404</p>
          <h1 className="m-0 text-[32px] leading-[1.1] font-semi tracking-[-0.8px]">Not found</h1>
          <p className="m-0 text-base leading-[1.45] text-bark">This page doesn&apos;t exist, or you don&apos;t have access to it.</p>
          <Link href="/app" className="btn-amber mt-3 inline-flex h-11 items-center rounded-lg px-5 text-[15px] font-semi no-underline">
            Go to your dashboard
          </Link>
        </div>
      </main>
    </div>
  );
}
