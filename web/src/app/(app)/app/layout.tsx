import type { ReactNode } from "react";
import { requirePageUser } from "@/lib/session";

export default async function AppLayout({ children }: { children: ReactNode }) {
  await requirePageUser("/app");
  return <>{children}</>;
}
