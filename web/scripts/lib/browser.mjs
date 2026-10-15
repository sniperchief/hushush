// A minimal cookie-keeping HTTP "browser" for scripts that drive a RUNNING
// app (`next start`) the way a real browser would, plus passkey sign-up and
// sign-in through the SOFTWARE authenticator (soft-passkey.mjs).
import { SoftPasskey } from "./soft-passkey.mjs";

export class Browser {
  /** @param {string} app origin, e.g. http://localhost:3000 */
  constructor(app) {
    this.app = app.replace(/\/+$/, "");
    /** @type {Map<string, string>} */
    this.jar = new Map();
  }

  get cookie() {
    return [...this.jar].map(([k, v]) => `${k}=${v}`).join("; ");
  }

  /** The session cookie alone (to persist a signed-in "browser"). */
  get session() {
    return [...this.jar].find(([k]) => k.endsWith("hh_session"))?.join("=") ?? "";
  }

  set session(c) {
    const i = c.indexOf("=");
    if (i > 0) this.jar.set(c.slice(0, i), c.slice(i + 1));
  }

  async req(method, path, body) {
    const res = await fetch(this.app + path, {
      method,
      redirect: "manual",
      headers: {
        ...(this.jar.size ? { cookie: this.cookie } : {}),
        ...(method !== "GET" ? { origin: this.app, "content-type": "application/json" } : {}),
      },
      body: method === "GET" ? undefined : JSON.stringify(body ?? {}),
    });
    for (const c of res.headers.getSetCookie()) {
      const [pair, ...attrs] = c.split(";");
      const i = pair.indexOf("=");
      const name = pair.slice(0, i);
      const expired = attrs.some((a) => /^\s*max-age=0\s*$/i.test(a));
      if (expired) this.jar.delete(name);
      else this.jar.set(name, pair.slice(i + 1));
    }
    const text = await res.text();
    let json;
    try {
      json = JSON.parse(text);
    } catch {
      json = undefined;
    }
    // `html`: the page text without React's `<!-- -->` text separators.
    return { status: res.status, text, html: text.replaceAll("<!-- -->", ""), json, location: res.headers.get("location") };
  }

  post(path, body) {
    return this.req("POST", path, body);
  }

  get(path) {
    return this.req("GET", path);
  }

  newPasskey() {
    return new SoftPasskey({ origin: this.app, rpId: new URL(this.app).hostname });
  }

  /**
   * Passkey sign-up, then the onboarding answer: "employer" = I pay a team,
   * "worker" = I get paid (accounts have no type). Returns the passkey (keep
   * it to sign in again).
   */
  async signUp(kind, displayName, passkey = this.newPasskey()) {
    const o = await this.post("/api/auth/passkey/register/options", { displayName });
    if (o.status !== 200) throw new Error(`register/options ${o.status} ${o.text}`);
    const v = await this.post("/api/auth/passkey/register/verify", { response: passkey.register(o.json) });
    if (v.status !== 201) throw new Error(`register/verify ${v.status} ${v.text}`);
    const i = await this.post("/api/account/intent", { intent: kind === "employer" ? "pay" : "get_paid" });
    if (i.status !== 200) throw new Error(`account/intent ${i.status} ${i.text}`);
    return passkey;
  }

  /** Passkey sign-in; returns true once signed in. */
  async signIn(passkey) {
    const o = await this.post("/api/auth/passkey/login/options");
    if (o.status !== 200) throw new Error(`login/options ${o.status} ${o.text}`);
    const v = await this.post("/api/auth/passkey/login/verify", { response: passkey.authenticate(o.json) });
    if (v.status !== 200) throw new Error(`login/verify ${v.status} ${v.text}`);
    return v.json.ok === true;
  }
}
