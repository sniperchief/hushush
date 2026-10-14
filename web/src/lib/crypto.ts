import "server-only";
import { createHash, randomBytes } from "node:crypto";

/** 32 random bytes, base64url — for session cookies, ceremony handles and worker links. */
export function newToken(): string {
  return randomBytes(32).toString("base64url");
}

/** Tokens are stored only as SHA-256 hex digests. */
export function hashToken(token: string): string {
  return createHash("sha256").update(token, "utf8").digest("hex");
}
