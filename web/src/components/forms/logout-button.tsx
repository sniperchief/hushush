"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { api } from "@/lib/client-api";
import { Button } from "../ui";

export function LogoutButton() {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  return (
    <Button
      variant="ghost"
      disabled={pending}
      onClick={async () => {
        setPending(true);
        await api("/api/auth/logout");
        router.push("/login");
        router.refresh();
      }}
    >
      Sign out
    </Button>
  );
}
