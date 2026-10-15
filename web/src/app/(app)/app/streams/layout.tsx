import type { ReactNode } from "react";
import { PlainShell } from "@/components/plain-shell";

export default function Layout({ children }: { children: ReactNode }) {
  return <PlainShell>{children}</PlainShell>;
}
