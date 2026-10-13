import { beforeAll, describe, expect, it } from "vitest";
import { payroll } from "@/lib/payroll";
import * as inviteRoute from "@/app/api/orgs/[orgId]/invitations/route";
import * as revokeRoute from "@/app/api/orgs/[orgId]/invitations/[invitationId]/route";
import * as memberRoute from "@/app/api/orgs/[orgId]/members/[userId]/route";
import * as streamsRoute from "@/app/api/orgs/[orgId]/streams/route";
import * as acceptRoute from "@/app/api/invitations/accept/route";
import * as releaseRoute from "@/app/api/claims/[claimId]/release/route";
import { call, claim, createOrg, fastStream, join, lastMail, lifecycle, newUser, sleep, tokenFrom, uniqueEmail, type StreamBody, type TestUser } from "./support/helpers";

let ownerA: TestUser, adminA: TestUser, worker: TestUser, ownerB: TestUser, workerB: TestUser;
let orgA: string, orgB: string;
let streamA: StreamBody;

beforeAll(async () => {
  [ownerA, adminA, worker, ownerB, workerB] = await Promise.all([
    newUser("ownerA", "Owner A"),
    newUser("adminA", "Admin A"),
    newUser("worker", "Worker"),
    newUser("ownerB", "Owner B"),
    newUser("workerB", "Worker B"),
  ]);
  orgA = await createOrg(ownerA, "Org A");
  orgB = await createOrg(ownerB, "Org B");
  await join(ownerA, orgA, adminA, "admin");
  await join(adminA, orgA, worker, "member");
  await join(ownerB, orgB, workerB, "member");
  streamA = await fastStream(adminA, orgA, worker);
  await sleep(1100);
});

describe("role permissions within an organization", () => {
  it("contributors cannot manage payroll or membership", async () => {
    const create = await call(streamsRoute.POST, "POST", { orgId: orgA }, {
      cookie: worker.cookie,
      body: { recipientUserId: worker.id, rateZec: "1", ratePeriodSecs: 3600, startAt: null, endAt: null },
    });
    expect(create.status).toBe(403);
    expect((await lifecycle(worker, streamA.id, "pause")).status).toBe(403);
    expect((await lifecycle(worker, streamA.id, "end")).status).toBe(403);
    const inv = await call(inviteRoute.POST, "POST", { orgId: orgA }, { cookie: worker.cookie, body: { email: uniqueEmail("x"), role: "member" } });
    expect(inv.status).toBe(403);
    const role = await call(memberRoute.PATCH, "PATCH", { orgId: orgA, userId: worker.id }, { cookie: worker.cookie, body: { role: "owner" } });
    expect(role.status).toBe(403);
    await expect(payroll.listMembers(worker.id, orgA)).rejects.toMatchObject({ status: 403 });
  });

  it("admins can invite contributors but not admins, and cannot change roles", async () => {
    const asAdmin = await call(inviteRoute.POST, "POST", { orgId: orgA }, { cookie: adminA.cookie, body: { email: uniqueEmail("a"), role: "admin" } });
    expect(asAdmin.status).toBe(403);
    const ok = await call(inviteRoute.POST, "POST", { orgId: orgA }, { cookie: adminA.cookie, body: { email: uniqueEmail("m"), role: "member" } });
    expect(ok.status).toBe(201);
    const promote = await call(memberRoute.PATCH, "PATCH", { orgId: orgA, userId: worker.id }, { cookie: adminA.cookie, body: { role: "admin" } });
    expect(promote.status).toBe(403);
  });

  it("only the recipient can claim; managers cannot claim on a worker's behalf", async () => {
    expect((await claim(adminA, streamA.id, "0.1")).status).toBe(403);
    expect((await claim(ownerA, streamA.id, "0.1")).status).toBe(403);
    expect((await claim(worker, streamA.id, "0.1")).status).toBe(201);
  });

  it("the last owner cannot be demoted, and members with streams cannot be removed", async () => {
    const demote = await call(memberRoute.PATCH, "PATCH", { orgId: orgA, userId: ownerA.id }, { cookie: ownerA.cookie, body: { role: "member" } });
    expect(demote.status).toBe(409);
    const remove = await call(memberRoute.DELETE, "DELETE", { orgId: orgA, userId: worker.id }, { cookie: ownerA.cookie });
    expect(remove.status).toBe(409);
  });
});

describe("invitations", () => {
  it("must be accepted by the invited email, once", async () => {
    const invitee = await newUser("inv");
    const stranger = await newUser("stranger");
    const r = await call(inviteRoute.POST, "POST", { orgId: orgA }, { cookie: ownerA.cookie, body: { email: invitee.email, role: "member" } });
    expect(r.status).toBe(201);
    const token = tokenFrom((await lastMail(invitee.email)).body, "invite");
    expect((await call(acceptRoute.POST, "POST", {}, { cookie: stranger.cookie, body: { token } })).status).toBe(403);
    expect((await call(acceptRoute.POST, "POST", {}, { body: { token } })).status).toBe(401);
    expect((await call(acceptRoute.POST, "POST", {}, { cookie: invitee.cookie, body: { token } })).status).toBe(200);
    expect((await call(acceptRoute.POST, "POST", {}, { cookie: invitee.cookie, body: { token } })).status).toBe(404);
    expect((await payroll.listOrgs(invitee.id)).map((o) => o.organization_id)).toContain(orgA);
    expect((await payroll.listOrgs(stranger.id))).toHaveLength(0);
  });

  it("revoked invitations cannot be accepted; duplicates are rejected", async () => {
    const invitee = await newUser("rev");
    const r = await call(inviteRoute.POST, "POST", { orgId: orgA }, { cookie: ownerA.cookie, body: { email: invitee.email, role: "member" } });
    const dup = await call(inviteRoute.POST, "POST", { orgId: orgA }, { cookie: ownerA.cookie, body: { email: invitee.email, role: "member" } });
    expect(dup.status).toBe(409);
    const token = tokenFrom((await lastMail(invitee.email)).body, "invite");
    const rv = await call(revokeRoute.DELETE, "DELETE", { orgId: orgA, invitationId: r.body.id as string }, { cookie: ownerA.cookie });
    expect(rv.status).toBe(200);
    expect((await call(acceptRoute.POST, "POST", {}, { cookie: invitee.cookie, body: { token } })).status).toBe(404);
  });

  it("re-checks the inviter's authority at acceptance time", async () => {
    const tempAdmin = await newUser("tmpadm");
    await join(ownerA, orgA, tempAdmin, "admin");
    const invitee = await newUser("late");
    await call(inviteRoute.POST, "POST", { orgId: orgA }, { cookie: tempAdmin.cookie, body: { email: invitee.email, role: "member" } });
    const token = tokenFrom((await lastMail(invitee.email)).body, "invite");
    // The admin is demoted before the invite is accepted.
    await call(memberRoute.PATCH, "PATCH", { orgId: orgA, userId: tempAdmin.id }, { cookie: ownerA.cookie, body: { role: "member" } });
    expect((await call(acceptRoute.POST, "POST", {}, { cookie: invitee.cookie, body: { token } })).status).toBe(403);
  });
});

describe("cross-organization isolation", () => {
  it("an owner of org B cannot see or touch org A", async () => {
    await expect(payroll.stream(ownerB.id, streamA.id)).rejects.toMatchObject({ status: 404 });
    await expect(payroll.listOrgStreams(ownerB.id, orgA)).rejects.toMatchObject({ status: 404 });
    await expect(payroll.listMembers(ownerB.id, orgA)).rejects.toMatchObject({ status: 404 });
    expect((await lifecycle(ownerB, streamA.id, "pause")).status).toBe(404);
    expect((await lifecycle(ownerB, streamA.id, "end")).status).toBe(404);
    expect((await claim(ownerB, streamA.id, "0.1")).status).toBe(404);
    const create = await call(streamsRoute.POST, "POST", { orgId: orgA }, {
      cookie: ownerB.cookie,
      body: { recipientUserId: ownerB.id, rateZec: "1", ratePeriodSecs: 3600, startAt: null, endAt: null },
    });
    expect(create.status).toBe(404);
    const inv = await call(inviteRoute.POST, "POST", { orgId: orgA }, { cookie: ownerB.cookie, body: { email: uniqueEmail("x"), role: "admin" } });
    expect(inv.status).toBe(404);
    const role = await call(memberRoute.PATCH, "PATCH", { orgId: orgA, userId: worker.id }, { cookie: ownerB.cookie, body: { role: "admin" } });
    expect(role.status).toBe(404);
    const rm = await call(memberRoute.DELETE, "DELETE", { orgId: orgA, userId: adminA.id }, { cookie: ownerB.cookie });
    expect(rm.status).toBe(404);
  });

  it("cannot pay people outside the organization or release other orgs' claims", async () => {
    const payOutsider = await call(streamsRoute.POST, "POST", { orgId: orgA }, {
      cookie: ownerA.cookie,
      body: { recipientUserId: workerB.id, rateZec: "1", ratePeriodSecs: 3600, startAt: null, endAt: null },
    });
    expect(payOutsider.status).toBe(422);
    const c = await claim(worker, streamA.id, "0.01");
    expect(c.status).toBe(201);
    const claimId = (c.body.claim as { id: string }).id;
    expect((await call(releaseRoute.POST, "POST", { claimId }, { cookie: ownerB.cookie })).status).toBe(404);
    expect((await call(releaseRoute.POST, "POST", { claimId }, { cookie: workerB.cookie })).status).toBe(404);
    // A contributor of org B sees only their own (empty) stream list.
    expect(await payroll.myStreams(workerB.id)).toHaveLength(0);
  });

  it("ignores malformed ids instead of erroring", async () => {
    expect((await lifecycle(ownerA, "not-a-uuid", "pause")).status).toBe(404);
    expect((await claim(worker, "../../etc", "1")).status).toBe(404);
  });
});
