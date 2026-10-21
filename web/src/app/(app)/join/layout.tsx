import type { ReactNode } from "react";

/** Worker links: the sign-up pages' frame (light, landing palette, no app header). */
export default function Layout({ children }: { children: ReactNode }) {
  return <div className="landing flex min-h-dvh flex-col bg-cream font-landing text-espresso">{children}</div>;
}
