import type { NextRequest } from "next/server";
import { json, requireUser, toResponse } from "@/lib/http";
import { payroll } from "@/lib/payroll";

type Ctx = { params: Promise<{ streamId: string }> };

/** The stream as its worker or its employer sees it (read-only). */
export async function GET(req: NextRequest, { params }: Ctx) {
  try {
    const user = await requireUser(req);
    return json(await payroll.flowStream(user.id, (await params).streamId));
  } catch (e) {
    return toResponse(e);
  }
}
