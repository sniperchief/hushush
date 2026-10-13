import "server-only";
import { randomUUID } from "node:crypto";
import { z } from "zod";
import type { SessionUser } from "./auth";
import { emailSchema } from "./auth";
import { hashToken, newToken } from "./crypto";
import { db } from "./db";
import { env } from "./env";
import { AppError, forbidden, notFound } from "./errors";
import { sendMail } from "./mail";
import { PayrollApiError, payroll, type Role } from "./payroll";

const INVITE_TTL_MS = 7 * 24 * 3600 * 1000;

export const inviteSchema = z.object({
  email: emailSchema,
  role: z.enum(["admin", "member"]),
});

export interface Invitation {
  id: string;
  email: string;
  role: "admin" | "member";
  invitedBy: string;
  createdAt: string;
  expiresAt: string;
}

/** The actor's role in `orgId`; outsiders get 404 (no existence leak). */
export async function roleIn(actor: string, orgId: string): Promise<{ role: Role; name: string }> {
  const m = (await payroll.listOrgs(actor)).find((o) => o.organization_id === orgId);
  if (!m) throw notFound("Organization not found.");
  return { role: m.role, name: m.organization_name };
}

async function requireManager(actor: string, orgId: string) {
  const r = await roleIn(actor, orgId);
  if (r.role === "member") throw forbidden("Only owners and admins can manage invitations.");
  return r;
}

export async function createInvitation(actor: SessionUser, orgId: string, input: z.input<typeof inviteSchema>): Promise<Invitation> {
  const { email, role } = inviteSchema.parse(input);
  const { role: actorRole, name } = await requireManager(actor.id, orgId);
  // Mirrors the engine rule (which is enforced again when the invite is accepted).
  if (role === "admin" && actorRole !== "owner") throw forbidden("Only owners can invite admins.");
  const members = await payroll.listMembers(actor.id, orgId);
  if (members.some((m) => m.email.toLowerCase() === email.toLowerCase())) {
    throw new AppError(409, "already_member", "That person is already a member.");
  }
  const token = newToken();
  const id = randomUUID();
  const expiresAt = new Date(Date.now() + INVITE_TTL_MS);
  try {
    await db().query(
      `INSERT INTO invitations (id, organization_id, email, role, invited_by, token_hash, expires_at)
       VALUES ($1, $2, $3, $4, $5, $6, $7)`,
      [id, orgId, email, role, actor.id, hashToken(token), expiresAt],
    );
  } catch (e) {
    if ((e as { code?: string }).code === "23505") {
      throw new AppError(409, "already_invited", "An invitation is already pending for that email.");
    }
    throw e;
  }
  await sendMail({
    to: email,
    subject: `${actor.displayName} invited you to ${name} on hushush`,
    text: `${actor.displayName} invited you to join ${name} on hushush as ${role === "admin" ? "an admin" : "a contributor"}.\n\nAccept the invitation:\n${env.appUrl}/invite/${token}\n\nThis link expires in 7 days.`,
  });
  return { id, email, role, invitedBy: actor.id, createdAt: new Date().toISOString(), expiresAt: expiresAt.toISOString() };
}

export async function listInvitations(actor: string, orgId: string): Promise<Invitation[]> {
  await requireManager(actor, orgId);
  const { rows } = await db().query(
    `SELECT id, email, role, invited_by, created_at, expires_at FROM invitations
     WHERE organization_id = $1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at > now()
     ORDER BY created_at DESC`,
    [orgId],
  );
  return rows.map((r) => ({
    id: r.id,
    email: r.email,
    role: r.role,
    invitedBy: r.invited_by,
    createdAt: new Date(r.created_at).toISOString(),
    expiresAt: new Date(r.expires_at).toISOString(),
  }));
}

export async function revokeInvitation(actor: string, orgId: string, invitationId: string): Promise<void> {
  await requireManager(actor, orgId);
  const { rowCount } = await db().query(
    `UPDATE invitations SET revoked_at = now()
     WHERE id = $1 AND organization_id = $2 AND accepted_at IS NULL AND revoked_at IS NULL`,
    [invitationId, orgId],
  );
  if (!rowCount) throw notFound("Invitation not found.");
}

export interface InvitationPreview {
  organizationName: string;
  email: string;
  role: "admin" | "member";
  inviterName: string;
  state: "open" | "accepted" | "revoked" | "expired";
}

export async function previewInvitation(token: string): Promise<InvitationPreview | null> {
  if (!token || token.length > 200) return null;
  const { rows } = await db().query(
    `SELECT o.name AS org_name, i.email, i.role, u.display_name AS inviter,
            i.accepted_at, i.revoked_at, (i.expires_at <= now()) AS expired
     FROM invitations i
     JOIN organizations o ON o.id = i.organization_id
     JOIN users u ON u.id = i.invited_by
     WHERE i.token_hash = $1`,
    [hashToken(token)],
  );
  const r = rows[0];
  if (!r) return null;
  return {
    organizationName: r.org_name,
    email: r.email,
    role: r.role,
    inviterName: r.inviter,
    state: r.accepted_at ? "accepted" : r.revoked_at ? "revoked" : r.expired ? "expired" : "open",
  };
}

/**
 * Accepts an invitation for the signed-in user. The invitation must be open
 * and addressed to the user's (verified) email. Membership is granted by the
 * payroll engine acting as the inviter, so the inviter's CURRENT role is
 * re-checked at acceptance time.
 */
export async function acceptInvitation(user: SessionUser, token: string): Promise<{ organizationId: string }> {
  const { rows } = await db().query(
    `SELECT id, organization_id, email, role, invited_by FROM invitations
     WHERE token_hash = $1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at > now()`,
    [hashToken(token)],
  );
  const inv = rows[0];
  if (!inv) throw notFound("This invitation is invalid, expired or already used.");
  if (String(inv.email).toLowerCase() !== user.email.toLowerCase()) {
    throw forbidden(`This invitation was sent to a different email address. Sign in as ${inv.email} to accept it.`);
  }
  try {
    await payroll.addMember(inv.invited_by, inv.organization_id, user.id, inv.role);
  } catch (e) {
    const alreadyMember = e instanceof PayrollApiError && e.code === "conflict";
    if (!alreadyMember) {
      if (e instanceof PayrollApiError && (e.status === 403 || e.status === 404)) {
        throw forbidden("The person who invited you can no longer add members. Ask for a new invitation.");
      }
      throw e;
    }
  }
  await db().query(
    `UPDATE invitations SET accepted_at = now(), accepted_by = $2
     WHERE id = $1 AND accepted_at IS NULL AND revoked_at IS NULL`,
    [inv.id, user.id],
  );
  return { organizationId: inv.organization_id };
}
