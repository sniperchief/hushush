import type { ReactNode } from "react";

/**
 * Signed-in area. Each page picks its frame: the dashboards use
 * DashboardShell; onboarding, stream pages and worker links use PlainShell.
 */
export default function AppGroupLayout({ children }: { children: ReactNode }) {
  return children;
}
