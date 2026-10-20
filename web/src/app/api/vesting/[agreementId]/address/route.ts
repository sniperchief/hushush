import { z } from "zod";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ agreementId: string }> };

// Shielded Unified Addresses are bech32m: lowercase letters and digits only.
// The payout wallet checks network and Orchard receiver. The web app passes
// the address through and never stores or logs it.
const schema = z.object({
  address: z
    .string()
    .trim()
    .min(10, "Enter a shielded Zcash testnet address.")
    .max(1000, "Enter a shielded Zcash testnet address.")
    .regex(/^[a-z0-9]+$/, "Enter a shielded Zcash testnet address."),
});

/** Sets the agreement's payout address. Once: it is then locked. */
export const POST = mutation(async (req, { params }: Ctx) => {
  const user = await requireUser(req);
  const { agreementId } = await params;
  const { address } = await readJson(req, schema);
  return json(await payroll.setVestingAddress(user.id, agreementId, address));
});
