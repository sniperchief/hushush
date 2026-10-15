import { z } from "zod";
import { intentSchema, setIntent } from "@/lib/auth";
import { json, mutation, readJson, requireUser } from "@/lib/http";

const schema = z.object({ intent: intentSchema });

/** Saves "How will you use hushush?" (which screen to start on). Changeable any time; grants nothing. */
export const POST = mutation(async (req) => {
  const user = await requireUser(req);
  const { intent } = await readJson(req, schema);
  await setIntent(user.id, intent);
  return json({ intent });
});
