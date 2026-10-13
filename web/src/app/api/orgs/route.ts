import { z } from "zod";
import { json, mutation, readJson, requireUser } from "@/lib/http";
import { payroll } from "@/lib/payroll";

export const POST = mutation(async (req) => {
  const user = await requireUser(req);
  const { name } = await readJson(req, z.object({ name: z.string().trim().min(1, "Enter a name.").max(200) }));
  return json(await payroll.createOrg(user.id, name), 201);
});
