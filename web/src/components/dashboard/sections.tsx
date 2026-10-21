import Link from "next/link";
import type { ReactNode } from "react";
import { WorkerStreamPanel } from "@/components/worker";
import { BackupEmailForm } from "@/components/email-login";
import { SectionTitle } from "@/components/ui";
import { initials, rangeLabel, streamState } from "@/lib/chart-data";
import { compactZec } from "@/lib/format";
import type { FlowStream, VestingView } from "@/lib/payroll";
import { pct, phaseLabel } from "@/lib/vesting";
import { Icon, type IconName } from "./icons";

// Building blocks of the dashboard pages (server components).

export const cardClass = "flex min-w-0 scroll-mt-4 flex-col gap-4 rounded-xl border border-linen bg-paper p-[18px] sm:px-6 sm:py-[22px]";

/** A dashboard card; with an `icon` its title is a section heading (icon tile, larger type). */
export function Card({
  id,
  title,
  icon,
  count,
  aside,
  children,
  className = "",
}: {
  id?: string;
  title: ReactNode;
  icon?: IconName;
  count?: number;
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  const h = id ? `${id}-title` : undefined;
  return (
    <section id={id} aria-labelledby={h} className={`${cardClass} ${className}`}>
      {icon ? (
        <SectionTitle id={h} icon={icon} count={count} action={aside}>
          {title}
        </SectionTitle>
      ) : (
        <div className="flex items-center justify-between gap-3">
          <h2 id={h} className="m-0 text-[17px] font-semi tracking-[-0.2px]">
            {title}
          </h2>
          {aside}
        </div>
      )}
      {children}
    </section>
  );
}

/**
 * The top of every dashboard page: an optional back link, the page title
 * (h1), a one-line description, an optional tag beside the title and the
 * page's main action on the right.
 */
export function PageHeader({
  title,
  lead,
  back,
  tag,
  action,
}: {
  title: ReactNode;
  lead?: ReactNode;
  back?: { href: string; label: string };
  tag?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-4 max-sm:flex-col max-sm:items-stretch">
      <div className="flex min-w-0 flex-col gap-1.5">
        {back && (
          <Link href={back.href} className="mb-1 inline-flex items-center gap-1.5 self-start text-sm text-bark no-underline hover:text-espresso">
            <span aria-hidden="true">←</span> {back.label}
          </Link>
        )}
        <div className="flex flex-wrap items-center gap-3">
          <h1 className="m-0 text-[26px] leading-[1.1] font-semi tracking-[-0.6px] break-words sm:text-[30px]">{title}</h1>
          {tag}
        </div>
        {lead && <p className="m-0 max-w-[68ch] text-sm leading-[1.45] text-bark sm:text-[15px]">{lead}</p>}
      </div>
      {action && <div className="flex items-center gap-2.5 max-sm:flex-col max-sm:items-stretch">{action}</div>}
    </div>
  );
}

/** The amber button-link for a page's main action ("New stream"). */
export function ActionLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <Link href={href} className="btn-amber inline-flex h-[42px] items-center justify-center gap-2 rounded-lg px-[18px] text-[15px] font-semi no-underline max-sm:h-12 max-sm:w-full">
      <Icon name="plus" size={15} strokeWidth={2.4} />
      {children}
    </Link>
  );
}

/** "View all →" style link with the amber underline. */
export function MoreLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <Link href={href} className="text-sm font-semi whitespace-nowrap text-espresso underline decoration-amber-deep decoration-2 underline-offset-4">
      {children}
    </Link>
  );
}

export function RangeChip({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex h-8 flex-none items-center gap-1.5 rounded-lg border border-linen bg-cream px-2.5 text-[13.5px] font-semi whitespace-nowrap">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#e89b01" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
        <rect x="3" y="5" width="18" height="16" rx="2" />
        <path d="M3 10h18M8 3v4M16 3v4" />
      </svg>
      {children}
    </span>
  );
}

export function Kpi({
  label,
  swatch,
  value,
  unit = "ZEC",
  sub,
  className = "",
}: {
  label: string;
  swatch?: string;
  value: ReactNode;
  unit?: string;
  sub?: ReactNode;
  className?: string;
}) {
  return (
    <div className={`flex min-w-0 flex-col gap-2 rounded-xl border border-linen bg-paper px-5 py-[18px] ${className}`}>
      <span className="flex items-center gap-2 text-[13px] text-bark">
        {swatch && <i aria-hidden="true" className="inline-block size-2.5 rounded-[3px]" style={{ background: swatch }} />}
        {label}
      </span>
      <span className="mono truncate text-[28px] leading-[1.1] font-medium tracking-[-0.6px]">
        {value} <span className="font-landing text-sm font-normal tracking-normal text-bark">{unit}</span>
      </span>
      {sub && <span className="text-[12.5px] text-bark">{sub}</span>}
    </div>
  );
}

/** Overview tile linking to a section page. */
export function SummaryTile({ href, icon, title, value, sub }: { href: string; icon: IconName; title: string; value: ReactNode; sub: ReactNode }) {
  return (
    <Link
      href={href}
      className="group flex min-w-0 flex-col gap-3 rounded-xl border border-linen bg-paper px-5 py-[18px] text-espresso no-underline transition-shadow hover:shadow-lift motion-reduce:transition-none"
    >
      <span className="flex items-center justify-between gap-3">
        <span className="flex items-center gap-2.5 text-[15px] font-semi">
          <span aria-hidden="true" className="flex size-8 flex-none items-center justify-center rounded-lg border border-linen bg-cream">
            <Icon name={icon} size={16} />
          </span>
          {title}
        </span>
        <span aria-hidden="true" className="text-bark transition-transform group-hover:translate-x-0.5 motion-reduce:transition-none">
          →
        </span>
      </span>
      <span className="text-[22px] leading-[1.15] font-semi tracking-[-0.4px]">{value}</span>
      <span className="text-[12.5px] leading-[1.4] text-bark">{sub}</span>
    </Link>
  );
}

export function EmptyBox({ children }: { children: ReactNode }) {
  return <div className="rounded-xl border-[1.5px] border-dashed border-stone px-5 py-10 text-center text-[15px] leading-normal text-bark">{children}</div>;
}

/** "Now" for this request (server components render once per request). */
export function requestTime(): number {
  return Math.floor(Date.now() / 1000);
}

export const sumOf = (streams: FlowStream[], f: (s: FlowStream) => number) => streams.reduce((a, s) => a + BigInt(f(s)), 0n);

export function SignInEmail({ email }: { email: string | null }) {
  return (
    <Card id="sign-in-email" title="Sign-in email" icon="email">
      <p className="m-0 text-sm leading-normal text-bark">
        {email ? (
          <>
            You can get a sign-in link at <b className="font-semi break-all text-espresso">{email}</b>, for example if you lose your passkey.
          </>
        ) : (
          "Add an email so you can still sign in with an email link if you lose your passkey."
        )}
      </p>
      <BackupEmailForm current={email} />
    </Card>
  );
}

// ---- tags and bars ------------------------------------------------------------

const tag = "inline-flex items-center rounded-full px-2.5 py-[3px] text-[12.5px] font-semi whitespace-nowrap";

export function VestingTag({ v }: { v: VestingView }) {
  const tone =
    v.phase === "vesting"
      ? "border border-linen"
      : v.phase === "fully_vested"
        ? "bg-espresso text-cream"
        : v.phase === "draft"
          ? "bg-honey-soft"
          : v.phase === "cancelled"
            ? "bg-mist"
            : "border border-linen bg-cream";
  return <span className={`${tag} ${tone}`}>{phaseLabel(v.phase)}</span>;
}

/** Vested / paid / on its way, as parts of the total (server figures). */
export function VestingBar({ v }: { v: VestingView }) {
  const b = v.balance;
  return (
    <div className="relative h-2 overflow-hidden rounded bg-mist" aria-hidden="true">
      <span className="absolute inset-y-0 left-0 bg-earned" style={{ width: `${pct(b.vested_zats, b.total_zats)}%` }} />
      <span className="absolute inset-y-0 left-0 bg-withdrawn" style={{ width: `${pct(b.paid_zats, b.total_zats)}%` }} />
      <span className="stripe-held absolute inset-y-0" style={{ left: `${pct(b.paid_zats, b.total_zats)}%`, width: `${pct(b.reserved_zats, b.total_zats)}%` }} />
    </div>
  );
}

export function StreamTag({ s }: { s: FlowStream }) {
  const state = streamState(s);
  if (state === "ended") return <span className={`${tag} bg-mist`}>Ended</span>;
  if (state === "withdrawing") return <span className={`${tag} bg-withdrawn-soft text-withdrawn-ink`}>Withdrawing</span>;
  return (
    <span className={`${tag} gap-1.5 border border-linen`}>
      <span aria-hidden="true" className="size-[7px] rounded-full bg-earned" />
      Active
    </span>
  );
}

// ---- worker ---------------------------------------------------------------------

export function WorkerStreamCard({ s }: { s: FlowStream }) {
  const address = s.address_locked ? (
    <span className="inline-flex items-center gap-1.5 self-start rounded-full border border-linen bg-cream px-[9px] py-[3px] text-[12.5px] whitespace-nowrap text-bark">
      <Icon name="lock" size={11} strokeWidth={2.4} />
      Payout address locked
    </span>
  ) : (
    <div className="flex items-center justify-between gap-2.5 rounded-lg border border-honey bg-honey-soft px-3.5 py-3">
      <span className="text-[13.5px] leading-[1.4]">Set your payout address to withdraw.</span>
      <MoreLink href={`/app/streams/${s.id}`}>Set it →</MoreLink>
    </div>
  );
  return (
    <section aria-labelledby={`stream-${s.id}`} className={cardClass}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <span aria-hidden="true" className="flex size-[38px] flex-none items-center justify-center rounded-[9px] bg-espresso text-[13px] font-semi text-cream">
            {initials(s.organization_name)}
          </span>
          <div className="min-w-0">
            <h2 id={`stream-${s.id}`} className="m-0 text-[17px] font-semi tracking-[-0.2px]">
              {s.organization_name} · <span className="mono font-medium">{compactZec(s.total_zec)}</span> ZEC
            </h2>
            <div className="mt-[3px] text-[12.5px] text-bark">
              {rangeLabel(s.start_at, s.ended_at ?? s.end_at)}
              {s.status === "ended" && " (ended)"}
            </div>
          </div>
        </div>
        <MoreLink href={`/app/streams/${s.id}`}>Details →</MoreLink>
      </div>
      <WorkerStreamPanel s={s} address={address} quietAddressHint />
    </section>
  );
}
