import { z } from "zod";

// Shielded Unified Addresses are bech32m: lowercase letters and digits only.
// Semantic checks (network, Orchard receiver, no transparent receiver) happen
// in the payout wallet; this only rejects obvious junk early. The address is
// passed through to payroll-api and never stored or logged by the web app.
export const addressSchema = z.object({
  address: z
    .string()
    .trim()
    .min(10, "Enter a shielded Zcash address.")
    .max(1000, "Enter a shielded Zcash address.")
    .regex(/^[a-z0-9]+$/, "Enter a shielded Zcash address."),
});
