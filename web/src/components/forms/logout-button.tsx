"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { api } from "@/lib/client-api";
import { Button } from "../ui";

/** Signs out. `className` replaces the default ghost-button look. */
export function LogoutButton({ redirectTo = "/", className, role }: { redirectTo?: string; className?: string; role?: "menuitem" }) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const onClick = async () => {
    setPending(true);
    await api("/api/auth/logout");
    router.push(redirectTo);
    router.refresh();
  };
  if (className) {
    return (
      <button type="button" role={role} disabled={pending} onClick={onClick} className={className}>
        Sign out
      </button>
    );
  }
  return (
    <Button variant="ghost" disabled={pending} onClick={onClick}>
      Sign out
    </Button>
  );
}
