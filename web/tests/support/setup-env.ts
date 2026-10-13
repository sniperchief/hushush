import { afterAll, inject } from "vitest";
import { closeDb } from "@/lib/db";

Object.assign(process.env, inject("testEnv"));

afterAll(async () => {
  await closeDb();
});
