"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { api } from "@/lib/client-api";
import { Alert, Button, Field, inputClass } from "../ui";

export function CreateOrgForm() {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  return (
    <form
      className="space-y-4"
      onSubmit={async (e) => {
        e.preventDefault();
        const name = String(new FormData(e.currentTarget).get("name") ?? "");
        setPending(true);
        setError(undefined);
        const r = await api<{ id: string }>("/api/orgs", "POST", { name });
        if (r.ok && r.data) {
          router.push(`/app/orgs/${r.data.id}`);
          router.refresh();
        } else {
          setPending(false);
          setError(r.error);
        }
      }}
    >
      <Field label="Organization name">
        <input name="name" required maxLength={200} placeholder="Acme Labs" className={inputClass} />
      </Field>
      {error && <Alert>{error}</Alert>}
      <Button type="submit" disabled={pending}>{pending ? "Creating…" : "Create organization"}</Button>
    </form>
  );
}

export function InviteForm({ orgId, canInviteAdmins }: { orgId: string; canInviteAdmins: boolean }) {
  const router = useRouter();
  const [msg, setMsg] = useState<{ tone: "red" | "green"; text: string }>();
  const [pending, setPending] = useState(false);
  return (
    <form
      className="grid gap-3 sm:grid-cols-[1fr_auto_auto] sm:items-end"
      onSubmit={async (e) => {
        e.preventDefault();
        const form = e.currentTarget;
        const fd = new FormData(form);
        setPending(true);
        setMsg(undefined);
        const r = await api(`/api/orgs/${orgId}/invitations`, "POST", { email: fd.get("email"), role: fd.get("role") });
        setPending(false);
        if (r.ok) {
          setMsg({ tone: "green", text: `Invitation sent to ${fd.get("email")}.` });
          form.reset();
          router.refresh();
        } else setMsg({ tone: "red", text: r.error ?? "Could not send invitation." });
      }}
    >
      <Field label="Email">
        <input name="email" type="email" required placeholder="contributor@example.com" className={inputClass} />
      </Field>
      <Field label="Role">
        <select name="role" className={inputClass} defaultValue="member">
          <option value="member">Contributor</option>
          {canInviteAdmins && <option value="admin">Admin</option>}
        </select>
      </Field>
      <Button type="submit" disabled={pending}>{pending ? "Sending…" : "Send invite"}</Button>
      {msg && <div className="sm:col-span-3"><Alert tone={msg.tone}>{msg.text}</Alert></div>}
    </form>
  );
}

export function RevokeInviteButton({ orgId, invitationId }: { orgId: string; invitationId: string }) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  return (
    <Button
      variant="ghost"
      disabled={pending}
      onClick={async () => {
        setPending(true);
        await api(`/api/orgs/${orgId}/invitations/${invitationId}`, "DELETE");
        router.refresh();
      }}
    >
      Revoke
    </Button>
  );
}

export function MemberControls({
  orgId,
  userId,
  role,
  actorRole,
  isSelf,
}: {
  orgId: string;
  userId: string;
  role: string;
  actorRole: string;
  isSelf: boolean;
}) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const canChangeRole = actorRole === "owner";
  const canRemove = !isSelf && (actorRole === "owner" || (actorRole === "admin" && role === "member"));
  if (!canChangeRole && !canRemove) return null;
  return (
    <div className="flex flex-wrap items-center justify-end gap-2">
      {canChangeRole && (
        <select
          aria-label="Role"
          className={`${inputClass} w-auto`}
          value={role}
          disabled={pending}
          onChange={async (e) => {
            setPending(true);
            setError(undefined);
            const r = await api(`/api/orgs/${orgId}/members/${userId}`, "PATCH", { role: e.target.value });
            setPending(false);
            if (!r.ok) setError(r.error);
            router.refresh();
          }}
        >
          <option value="owner">Owner</option>
          <option value="admin">Admin</option>
          <option value="member">Contributor</option>
        </select>
      )}
      {canRemove && (
        <Button
          variant="ghost"
          disabled={pending}
          onClick={async () => {
            if (!confirm("Remove this member from the organization?")) return;
            setPending(true);
            setError(undefined);
            const r = await api(`/api/orgs/${orgId}/members/${userId}`, "DELETE");
            setPending(false);
            if (!r.ok) setError(r.error);
            router.refresh();
          }}
        >
          Remove
        </Button>
      )}
      {error && <p className="w-full text-right text-xs text-bad">{error}</p>}
    </div>
  );
}

export function AcceptInviteButton({ token }: { token: string }) {
  const router = useRouter();
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  return (
    <div className="space-y-3">
      {error && <Alert>{error}</Alert>}
      <Button
        disabled={pending}
        onClick={async () => {
          setPending(true);
          setError(undefined);
          const r = await api<{ organizationId: string }>("/api/invitations/accept", "POST", { token });
          if (r.ok && r.data) {
            router.push(`/app/orgs/${r.data.organizationId}`);
            router.refresh();
          } else {
            setPending(false);
            setError(r.error);
          }
        }}
      >
        {pending ? "Joining…" : "Accept invitation"}
      </Button>
    </div>
  );
}
