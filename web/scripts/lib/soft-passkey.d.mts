// Types for soft-passkey.mjs (a SOFTWARE authenticator for tests and smoke runs).
type Json = Record<string, unknown>;

export declare class SoftPasskey {
  constructor(o: { origin: string; rpId: string; userVerified?: boolean });
  readonly id: string;
  counter: number;
  userVerified: boolean;
  userHandle: string | undefined;
  /** `options`: PublicKeyCredentialCreationOptionsJSON. */
  register(options: Json, o?: { origin?: string }): Json;
  /** `options`: PublicKeyCredentialRequestOptionsJSON. */
  authenticate(options: Json, o?: { origin?: string; counter?: number }): Json;
}
