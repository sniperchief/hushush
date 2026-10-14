"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { api } from "@/lib/client-api";
import { Alert, Button } from "./ui";

/** A signed-in worker accepts the stream behind a worker link. */
export function JoinButton({ token }: { token: string }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  return (
    <div className="space-y-3">
      <Button
        className="w-full"
        disabled={pending}
        onClick={async () => {
          setPending(true);
          setError(undefined);
          const r = await api<{ streamId: string }>(`/api/join/${token}`);
          if (!r.ok || !r.data) {
            setPending(false);
            return setError(r.error);
          }
          router.push(`/app/streams/${r.data.streamId}`);
          router.refresh();
        }}
      >
        {pending ? "Joining…" : "Accept this stream"}
      </Button>
      {error && <Alert>{error}</Alert>}
    </div>
  );
}
