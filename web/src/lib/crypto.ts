import "server-only";
import { createHash, randomBytes, scrypt, timingSafeEqual } from "node:crypto";

/** 32 random bytes, base64url — for session cookies and email/invite links. */
export function newToken(): string {
  return randomBytes(32).toString("base64url");
}

/** Tokens are stored only as SHA-256 hex digests. */
export function hashToken(token: string): string {
  return createHash("sha256").update(token, "utf8").digest("hex");
}

// scrypt parameters (OWASP: N=2^17, r=8, p=1 is the current recommendation).
const N = 2 ** 17;
const R = 8;
const P = 1;
const KEYLEN = 64;
const MAXMEM = 256 * 1024 * 1024;

function scryptAsync(password: string, salt: Buffer, n: number, r: number, p: number, keylen: number): Promise<Buffer> {
  return new Promise((resolve, reject) =>
    scrypt(password.normalize("NFKC"), salt, keylen, { N: n, r, p, maxmem: MAXMEM }, (err, key) =>
      err ? reject(err) : resolve(key),
    ),
  );
}

/** Hashes a password with Node's built-in scrypt and a random 16-byte salt. */
export async function hashPassword(password: string): Promise<string> {
  const salt = randomBytes(16);
  const key = await scryptAsync(password, salt, N, R, P, KEYLEN);
  return `scrypt$${N}$${R}$${P}$${salt.toString("base64")}$${key.toString("base64")}`;
}

/** Constant-time verification. Returns false for malformed hashes. */
export async function verifyPassword(password: string, stored: string): Promise<boolean> {
  const parts = stored.split("$");
  if (parts.length !== 6 || parts[0] !== "scrypt") return false;
  const [n, r, p] = [Number(parts[1]), Number(parts[2]), Number(parts[3])];
  const salt = Buffer.from(parts[4] ?? "", "base64");
  const expected = Buffer.from(parts[5] ?? "", "base64");
  if (!Number.isInteger(n) || !Number.isInteger(r) || !Number.isInteger(p) || expected.length === 0) return false;
  const key = await scryptAsync(password, salt, n, r, p, expected.length);
  return timingSafeEqual(key, expected);
}

/** A fixed hash used to keep login timing uniform for unknown emails. */
export const DUMMY_PASSWORD_HASH =
  "scrypt$131072$8$1$AAAAAAAAAAAAAAAAAAAAAA==$" + Buffer.alloc(64).toString("base64");
