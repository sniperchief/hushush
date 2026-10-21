import type { ReactNode } from "react";
import type { NavItem } from "@/components/dashboard/nav";
import { DashboardShell } from "@/components/dashboard/shell";
import { companyOf, employerHomeOf, myVestingOf, orgVestingOf, workerStreamsOf } from "@/lib/dashboard";
import { currentUser } from "@/lib/session";

const ACCOUNT: NavItem[] = [{ href: "/app/account", label: "Sign-in email", icon: "email" }];

/**
 * Every signed-in page under /app sits in the dashboard frame: header,
 * sidebar (drawer and bottom tabs on phones), and the page itself. Each
 * sidebar item is its own page; the frame highlights the current one.
 *
 * Signed out, or still onboarding (no answer to "pay or get paid", or an
 * employer without a company yet), the page renders on its own: it redirects
 * to sign-in or shows the onboarding step.
 */
export default async function AppLayout({ children }: { children: ReactNode }) {
  const user = await currentUser();
  if (!user?.intent) return children;

  if (user.intent === "pay") {
    const company = await companyOf(user.id);
    if (!company) return children;
    const [home, vesting] = await Promise.all([employerHomeOf(user.id, company.id), orgVestingOf(user.id, company.id)]);
    const waiting = home.offers.filter((o) => o.status === "open").length;
    const nav: NavItem[] = [
      { href: "/app", label: "Overview", icon: "overview" },
      { href: "/app/streams", label: "Streams", icon: "streams", count: home.streams.length },
      { href: "/app/streams#waiting", label: "Waiting to join", icon: "waiting", count: waiting, alert: true },
      { href: "/app/vesting", label: "Vesting", icon: "vesting", count: vesting.length },
      { href: "/app/payments", label: "Payments", icon: "payments" },
      { href: "/app/wallet", label: "Payout wallet", icon: "treasury" },
    ];
    const tabs: NavItem[] = [
      { href: "/app", label: "Overview", icon: "overview" },
      { href: "/app/streams", label: "Streams", icon: "streams", count: waiting, alert: true },
      { href: "/app/vesting", label: "Vesting", icon: "vesting" },
      { href: "/app/payments", label: "Payments", icon: "payments" },
      { href: "/app/wallet", label: "Wallet", icon: "treasury" },
    ];
    return (
      <DashboardShell role="employer" displayName={user.displayName} company={company.name} nav={nav} account={ACCOUNT} tabs={tabs}>
        {children}
      </DashboardShell>
    );
  }

  const [streams, vesting] = await Promise.all([workerStreamsOf(user.id), myVestingOf(user.id)]);
  const missing = streams.filter((s) => !s.address_locked).length;
  const nav: NavItem[] = [
    { href: "/app", label: "Overview", icon: "overview" },
    { href: "/app/streams", label: "My streams", icon: "streams", count: streams.length },
    { href: "/app/withdrawals", label: "Withdrawals", icon: "payments" },
    { href: "/app/vesting", label: "Vesting", icon: "vesting", count: vesting.length },
    { href: "/app/streams#addresses", label: "Payout addresses", icon: "lock", count: missing, alert: true },
  ];
  const tabs: NavItem[] = [
    { href: "/app", label: "Overview", icon: "overview" },
    { href: "/app/streams", label: "Streams", icon: "streams", count: missing, alert: true },
    { href: "/app/withdrawals", label: "Withdrawals", icon: "payments" },
    { href: "/app/vesting", label: "Vesting", icon: "vesting" },
  ];
  return (
    <DashboardShell role="worker" displayName={user.displayName} nav={nav} account={ACCOUNT} tabs={tabs}>
      {children}
    </DashboardShell>
  );
}
