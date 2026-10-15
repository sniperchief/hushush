import type { ReactNode } from "react";

/** Sign up, sign in and email links: light-only pages in the landing palette, without the app header. */
export default function AuthLayout({ children }: { children: ReactNode }) {
  return (
    <div className="landing force-light flex min-h-dvh flex-col bg-cream font-landing text-espresso">{children}</div>
  );
}
