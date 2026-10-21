import Link from "next/link";
import type { ReactNode } from "react";
import { initials } from "@/lib/chart-data";
import { Wordmark } from "../landing/ui";
import { SwitchIntent } from "../onboarding";
import { BottomTabs, MobileMenu, SidebarLinks, type NavItem } from "./nav";
import { ProfileMenu } from "./profile-menu";

export interface ShellProps {
  role: "employer" | "worker";
  displayName: string;
  company?: string;
  nav: NavItem[];
  account: NavItem[];
  tabs: NavItem[];
  children: ReactNode;
}

/** "Getting paid too?" / "Paying people too?" card at the bottom of the sidebar. */
function SwitchCard({ role }: { role: ShellProps["role"] }) {
  const employer = role === "employer";
  return (
    <div className="flex flex-col gap-2 rounded-[10px] border border-linen bg-chalk p-3.5">
      <span className="text-[13.5px] font-semi">{employer ? "Getting paid too?" : "Paying people too?"}</span>
      <span className="text-[12.5px] leading-[1.45] text-bark">
        {employer ? "Switch to the worker view to see streams paid to you." : "Switch to the employer view to stream pay to your own team."}
      </span>
      <SwitchIntent
        to={employer ? "get_paid" : "pay"}
        className="cursor-pointer self-start border-0 bg-transparent p-0 text-left text-[13.5px] font-semi text-espresso underline decoration-amber-deep decoration-2 underline-offset-4 disabled:cursor-wait disabled:opacity-60"
      >
        {employer ? "Get paid through hushush →" : "Start paying people →"}
      </SwitchIntent>
    </div>
  );
}

/** Top bar, sidebar (desktop) or drawer + bottom tabs (phones), around every dashboard page. */
export function DashboardShell({ role, displayName, company, nav, account, tabs, children }: ShellProps) {
  const sidebar = (
    <>
      <SidebarLinks items={nav} account={account} />
      <div className="mt-auto pt-4">
        <SwitchCard role={role} />
      </div>
    </>
  );
  return (
    <div className="landing force-light flex min-h-dvh flex-col bg-cream font-landing text-espresso">
      <header className="flex h-[60px] flex-none items-center gap-3 border-b border-linen bg-paper px-4 min-[900px]:h-[68px] min-[900px]:gap-5 min-[900px]:px-6">
        <div className="min-[900px]:hidden">
          <MobileMenu>{sidebar}</MobileMenu>
        </div>
        <Link href="/" aria-label="hushush home" className="flex items-center">
          <Wordmark className="h-[22px] min-[900px]:h-[26px]" alt="" preload />
        </Link>
        {company && (
          <>
            <span aria-hidden="true" className="ml-7 h-7 w-px bg-linen max-[899px]:hidden" />
            <span className="flex min-w-0 items-center gap-2.5 text-[15px] font-semi max-[899px]:hidden">
              <span aria-hidden="true" className="flex size-[26px] flex-none items-center justify-center rounded-md bg-espresso text-[10.5px] text-cream">
                {initials(company)}
              </span>
              <span className="truncate">{company}</span>
            </span>
          </>
        )}
        <div className="ml-auto flex min-w-0 items-center gap-2.5 min-[900px]:gap-3.5">
          <span className="inline-flex items-center gap-[7px] rounded-full bg-honey px-2 py-1 text-[11.5px] font-semi min-[900px]:px-2.5 min-[900px]:py-[5px] min-[900px]:text-[12.5px]">
            <span aria-hidden="true" className="size-1.5 rounded-xs bg-espresso max-[899px]:hidden" />
            <span className="min-[900px]:hidden">Testnet</span>
            <span className="max-[899px]:hidden">Zcash testnet</span>
          </span>
          <ProfileMenu name={displayName} company={company} role={role} />
        </div>
      </header>

      <div className="flex flex-1">
        <aside className="w-[260px] flex-none border-r border-linen bg-paper max-[899px]:hidden">
          <nav aria-label="Dashboard" className="sticky top-0 flex h-dvh flex-col px-4 py-5">
            {sidebar}
          </nav>
        </aside>
        <main className="flex min-w-0 flex-1 flex-col gap-4 px-4 pt-6 pb-28 min-[900px]:gap-5 min-[900px]:px-9 min-[900px]:pt-8 min-[900px]:pb-14">
          {children}
        </main>
      </div>
      <BottomTabs tabs={tabs} />
    </div>
  );
}
