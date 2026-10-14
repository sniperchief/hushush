// *** SOFTWARE passkey authenticator — for tests and the smoke script only. ***
//
// Produces genuine WebAuthn responses (ES256 key pair, "none" attestation,
// user-present + user-verified flags, signature counter) so the app's REAL
// verification code (@simplewebauthn/server) runs unchanged. It stands in for
// Face ID / Windows Hello; it is never used by the app itself.
import { createHash, generateKeyPairSync, randomBytes, sign } from "node:crypto";
import { isoCBOR } from "@simplewebauthn/server/helpers";

const b64u = (b) => Buffer.from(b).toString("base64url");
const sha256 = (b) => createHash("sha256").update(b).digest();

const FLAG_UP = 0x01;
const FLAG_UV = 0x04;
const FLAG_AT = 0x40;

export class SoftPasskey {
  /** @param {{ origin: string, rpId: string, userVerified?: boolean }} o */
  constructor({ origin, rpId, userVerified = true }) {
    this.origin = origin;
    this.rpId = rpId;
    this.userVerified = userVerified;
    this.credentialId = randomBytes(32);
    const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
    this.privateKey = privateKey;
    this.jwk = publicKey.export({ format: "jwk" });
    this.counter = 0;
    /** @type {string | undefined} */
    this.userHandle = undefined;
  }

  get id() {
    return b64u(this.credentialId);
  }

  flags(extra = 0) {
    return FLAG_UP | (this.userVerified ? FLAG_UV : 0) | extra;
  }

  authData(flags, attested) {
    const counter = Buffer.alloc(4);
    counter.writeUInt32BE(this.counter);
    return Buffer.concat([sha256(this.rpId), Buffer.from([flags]), counter, attested ?? Buffer.alloc(0)]);
  }

  clientData(type, challenge, origin = this.origin) {
    return Buffer.from(JSON.stringify({ type, challenge, origin, crossOrigin: false }));
  }

  /** navigator.credentials.create() for `options` (PublicKeyCredentialCreationOptionsJSON). */
  register(options, { origin } = {}) {
    this.userHandle = options.user.id;
    const cose = new Map([
      [1, 2], // kty: EC2
      [3, -7], // alg: ES256
      [-1, 1], // crv: P-256
      [-2, new Uint8Array(Buffer.from(this.jwk.x, "base64url"))],
      [-3, new Uint8Array(Buffer.from(this.jwk.y, "base64url"))],
    ]);
    const idLen = Buffer.alloc(2);
    idLen.writeUInt16BE(this.credentialId.length);
    const attested = Buffer.concat([Buffer.alloc(16), idLen, this.credentialId, Buffer.from(isoCBOR.encode(cose))]);
    const attestationObject = isoCBOR.encode(
      new Map([
        ["fmt", "none"],
        ["attStmt", new Map()],
        ["authData", new Uint8Array(this.authData(this.flags(FLAG_AT), attested))],
      ]),
    );
    return {
      id: this.id,
      rawId: this.id,
      type: "public-key",
      response: {
        clientDataJSON: b64u(this.clientData("webauthn.create", options.challenge, origin)),
        attestationObject: b64u(attestationObject),
        transports: ["internal"],
      },
      clientExtensionResults: {},
      authenticatorAttachment: "platform",
    };
  }

  /** navigator.credentials.get() for `options` (PublicKeyCredentialRequestOptionsJSON). */
  authenticate(options, { origin, counter } = {}) {
    this.counter = counter ?? this.counter + 1;
    const authData = this.authData(this.flags());
    const clientDataJSON = this.clientData("webauthn.get", options.challenge, origin);
    const signature = sign("sha256", Buffer.concat([authData, sha256(clientDataJSON)]), this.privateKey);
    return {
      id: this.id,
      rawId: this.id,
      type: "public-key",
      response: {
        clientDataJSON: b64u(clientDataJSON),
        authenticatorData: b64u(authData),
        signature: b64u(signature),
        userHandle: this.userHandle,
      },
      clientExtensionResults: {},
      authenticatorAttachment: "platform",
    };
  }
}
