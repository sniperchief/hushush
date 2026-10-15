import "server-only";
import {
  generateAuthenticationOptions,
  generateRegistrationOptions,
  verifyAuthenticationResponse,
  verifyRegistrationResponse,
  type AuthenticationResponseJSON,
  type PublicKeyCredentialCreationOptionsJSON,
  type PublicKeyCredentialRequestOptionsJSON,
  type RegistrationResponseJSON,
} from "@simplewebauthn/server";
import { randomBytes } from "node:crypto";
import { createAccount, createSession } from "./auth";
import { hashToken, newToken } from "./crypto";
import { db } from "./db";
import { env } from "./env";
import { AppError } from "./errors";

// Passkey (WebAuthn) sign-up and sign-in. No passwords, no e-mail.
//
// Each ceremony has a server-side challenge row, single-use and valid for 5
// minutes, reached through a random handle in an HttpOnly cookie (only the
// handle's hash is stored). User verification (Face ID, fingerprint, Windows
// Hello, device PIN) is required. Exactly one passkey per account.

const CHALLENGE_TTL_MS = 5 * 60 * 1000;
const RP_NAME = "hushush";

/** Cookie carrying the pending ceremony's handle. */
export function ceremonyCookieName(): string {
  return env.appUrl.startsWith("https://") ? "__Host-hh_webauthn" : "hh_webauthn";
}
export const CEREMONY_COOKIE_MAX_AGE = CHALLENGE_TTL_MS / 1000;

const authFailed = (m = "Passkey verification failed. Try again.") => new AppError(401, "passkey_failed", m);

async function storeChallenge(purpose: "register" | "login", challenge: string, payload: object): Promise<string> {
  const handle = newToken();
  // Opportunistic cleanup keeps abandoned ceremonies from accumulating.
  await db().query("DELETE FROM webauthn_challenges WHERE expires_at < now() - interval '1 hour'");
  await db().query(
    "INSERT INTO webauthn_challenges (handle_hash, purpose, challenge, payload, expires_at) VALUES ($1, $2, $3, $4, $5)",
    [hashToken(handle), purpose, challenge, JSON.stringify(payload), new Date(Date.now() + CHALLENGE_TTL_MS)],
  );
  return handle;
}

/** Consumes a ceremony: single use even when verification then fails. */
async function consumeChallenge<P>(handle: string | undefined, purpose: "register" | "login"): Promise<{ challenge: string; payload: P }> {
  if (!handle || handle.length > 200) throw authFailed("Your passkey request expired. Try again.");
  const { rows } = await db().query<{ challenge: string; payload: P }>(
    `UPDATE webauthn_challenges SET used_at = now()
     WHERE handle_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > now()
     RETURNING challenge, payload`,
    [hashToken(handle), purpose],
  );
  if (!rows[0]) throw authFailed("Your passkey request expired. Try again.");
  return rows[0];
}

interface RegisterPayload {
  displayName: string;
  webauthnUserId: string;
}

export async function registrationOptions(
  displayName: string,
): Promise<{ options: PublicKeyCredentialCreationOptionsJSON; handle: string }> {
  const userID = randomBytes(32);
  const options = await generateRegistrationOptions({
    rpName: RP_NAME,
    rpID: env.rpId,
    userName: displayName,
    userDisplayName: displayName,
    userID,
    attestationType: "none",
    authenticatorSelection: { residentKey: "required", userVerification: "required" },
  });
  const payload: RegisterPayload = { displayName, webauthnUserId: userID.toString("base64url") };
  return { options, handle: await storeChallenge("register", options.challenge, payload) };
}

/** Verifies a new passkey, creates the account and opens a session. */
export async function verifyRegistration(
  handle: string | undefined,
  response: RegistrationResponseJSON,
): Promise<{ userId: string; session: { token: string; expiresAt: Date } }> {
  const { challenge, payload } = await consumeChallenge<RegisterPayload>(handle, "register");
  let verified;
  try {
    verified = await verifyRegistrationResponse({
      response,
      expectedChallenge: challenge,
      expectedOrigin: env.appOrigin,
      expectedRPID: env.rpId,
      requireUserVerification: true,
    });
  } catch {
    throw authFailed();
  }
  if (!verified.verified) throw authFailed();
  const info = verified.registrationInfo;
  const cred = info.credential;

  const { rowCount } = await db().query("SELECT 1 FROM passkeys WHERE credential_id = $1", [cred.id]);
  if (rowCount) throw new AppError(409, "passkey_exists", "This passkey already belongs to an account. Sign in instead.");

  const client = await db().connect();
  let userId: string;
  try {
    await client.query("BEGIN");
    userId = await createAccount(client, payload.displayName);
    await client.query(
      `INSERT INTO passkeys (credential_id, user_id, webauthn_user_id, public_key, counter, transports, device_type, backed_up)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8)`,
      [
        cred.id,
        userId,
        payload.webauthnUserId,
        Buffer.from(cred.publicKey),
        cred.counter,
        cred.transports ?? [],
        info.credentialDeviceType,
        info.credentialBackedUp,
      ],
    );
    await client.query("COMMIT");
  } catch (e) {
    await client.query("ROLLBACK");
    // Lost a race with a concurrent sign-up presenting the same credential.
    if ((e as { code?: string }).code === "23505") {
      throw new AppError(409, "passkey_exists", "This passkey already belongs to an account. Sign in instead.");
    }
    throw e;
  } finally {
    client.release();
  }
  return { userId, session: await createSession(userId) };
}

export async function authenticationOptions(): Promise<{ options: PublicKeyCredentialRequestOptionsJSON; handle: string }> {
  // Discoverable credentials: the authenticator offers this site's passkeys.
  const options = await generateAuthenticationOptions({ rpID: env.rpId, userVerification: "required" });
  return { options, handle: await storeChallenge("login", options.challenge, {}) };
}

/** Verifies a passkey assertion and opens a session. */
export async function verifyAuthentication(
  handle: string | undefined,
  response: AuthenticationResponseJSON,
): Promise<{ userId: string; session: { token: string; expiresAt: Date } }> {
  const { challenge } = await consumeChallenge<object>(handle, "login");
  const { rows } = await db().query<{
    user_id: string;
    webauthn_user_id: string;
    public_key: Buffer;
    counter: string;
    transports: string[];
  }>(
    `SELECT user_id, webauthn_user_id, public_key, counter, transports FROM passkeys WHERE credential_id = $1`,
    [String(response?.id ?? "")],
  );
  const pk = rows[0];
  if (!pk) throw authFailed("This passkey isn't registered with hushush.");
  if (response.response?.userHandle && response.response.userHandle !== pk.webauthn_user_id) throw authFailed();
  let verified;
  try {
    verified = await verifyAuthenticationResponse({
      response,
      expectedChallenge: challenge,
      expectedOrigin: env.appOrigin,
      expectedRPID: env.rpId,
      requireUserVerification: true,
      credential: {
        id: response.id,
        publicKey: new Uint8Array(pk.public_key),
        counter: Number(pk.counter),
        transports: pk.transports,
      },
    });
  } catch {
    throw authFailed();
  }
  if (!verified.verified) throw authFailed();
  // Compare-and-set on the signature counter (a cloned authenticator, or a
  // concurrent replay of one assertion, loses).
  const { rowCount } = await db().query(
    "UPDATE passkeys SET counter = $1, last_used_at = now() WHERE credential_id = $2 AND counter = $3",
    [verified.authenticationInfo.newCounter, response.id, pk.counter],
  );
  if (!rowCount) throw authFailed();
  return { userId: pk.user_id, session: await createSession(pk.user_id) };
}
