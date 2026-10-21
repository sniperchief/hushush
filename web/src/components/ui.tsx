import type { ComponentProps, ReactNode } from "react";
import { Icon, type IconName } from "./dashboard/icons";

// Shared building blocks for the signed-in screens, in the product palette
// (warm, light only): the same cards, tags and figures as the dashboards.

export function cx(...c: (string | false | null | undefined)[]): string {
  return c.filter(Boolean).join(" ");
}

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  return <section className={cx("min-w-0 rounded-xl border border-linen bg-paper p-[18px] sm:px-6 sm:py-[22px]", className)}>{children}</section>;
}

/** A section's heading: an icon tile (optional), the title, and an action on the right. */
export function SectionTitle({ icon, children, count, action, id }: { icon?: IconName; children: ReactNode; count?: number; action?: ReactNode; id?: string }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex min-w-0 items-center gap-3">
        {icon && (
          <span aria-hidden="true" className="flex size-9 flex-none items-center justify-center rounded-lg border border-linen bg-cream text-espresso">
            <Icon name={icon} size={18} />
          </span>
        )}
        <h2 id={id} className="m-0 text-[19px] leading-[1.2] font-semi tracking-[-0.3px]">
          {children}
        </h2>
        {count !== undefined && <span className="rounded-full border border-linen bg-cream px-2 py-0.5 text-xs font-semi">{count}</span>}
      </div>
      {action}
    </div>
  );
}

export function CardTitle({ children, action, icon }: { children: ReactNode; action?: ReactNode; icon?: IconName }) {
  return (
    <div className="mb-4">
      <SectionTitle icon={icon} action={action}>
        {children}
      </SectionTitle>
    </div>
  );
}

const tones = {
  neutral: "bg-mist text-espresso",
  /** Running: "Active", "Vesting". */
  green: "border border-linen bg-paper text-espresso",
  /** Needs attention: drafts, "Checking". */
  amber: "border border-honey bg-honey-soft text-espresso",
  red: "bg-failed-bg text-failed-ink",
  /** Not started yet, in the cliff. */
  accent: "bg-honey text-espresso",
  /** On its way: "Withdrawing", "Confirming n/10". */
  withdrawing: "bg-withdrawn-soft text-withdrawn-ink",
  paid: "bg-espresso text-cream",
} as const;

export function Badge({ children, tone = "neutral", title }: { children: ReactNode; tone?: keyof typeof tones; title?: string }) {
  return (
    <span title={title} className={cx("inline-flex items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-[3px] text-[12.5px] font-semi", tones[tone])}>
      {tone === "green" && <span aria-hidden="true" className="size-[7px] rounded-full bg-earned" />}
      {children}
    </span>
  );
}

/** Amber is the only filled button colour; secondary is an espresso outline. */
export function Button({ variant = "primary", className, ...props }: ComponentProps<"button"> & { variant?: "primary" | "secondary" | "danger" | "ghost" }) {
  const styles = {
    primary: "btn-amber",
    secondary: "border border-espresso bg-transparent text-espresso hover:bg-paper",
    danger: "border border-danger-line bg-danger-soft text-failed-ink hover:border-failed-line",
    ghost: "text-bark hover:bg-mist hover:text-espresso",
  }[variant];
  return (
    <button
      {...props}
      className={cx(
        "inline-flex h-[42px] cursor-pointer items-center justify-center gap-2 rounded-lg px-[18px] text-[15px] font-semi transition-[transform,background-color] duration-150 active:scale-[.98] disabled:cursor-not-allowed disabled:opacity-60 disabled:active:scale-100 motion-reduce:transition-none",
        styles,
        className,
      )}
    />
  );
}

export function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-sm font-semi text-espresso">{label}</span>
      {children}
      {hint && <span className="mt-1.5 block text-[12.5px] leading-[1.45] text-bark">{hint}</span>}
    </label>
  );
}

export const inputClass =
  "block w-full h-11 rounded-lg border border-linen bg-paper px-3.5 text-[15px] text-espresso placeholder:text-placeholder outline-none transition-[border-color,box-shadow] focus:border-amber-deep focus:ring-2 focus:ring-amber/25";

export function Alert({ tone = "red", children }: { tone?: "red" | "green" | "amber" | "neutral"; children: ReactNode }) {
  const t = {
    red: "border-failed-line bg-failed-bg text-failed-ink",
    green: "border-honey bg-honey-soft text-espresso",
    amber: "border-honey bg-honey-band text-espresso",
    neutral: "border-linen bg-cream text-bark",
  }[tone];
  return (
    <div role={tone === "red" ? "alert" : "status"} className={cx("rounded-lg border px-3.5 py-3 text-sm leading-[1.45]", t)}>
      {children}
    </div>
  );
}

export function Stat({ label, value, unit = "ZEC", hint, emphasis }: { label: string; value: string; unit?: string; hint?: string; emphasis?: boolean }) {
  return (
    <div className={cx("flex min-w-0 flex-col gap-1.5 rounded-xl border px-4 py-3.5", emphasis ? "border-honey bg-honey-soft" : "border-linen bg-paper")}>
      <div className="text-[13px] text-bark">{label}</div>
      <div className="mono truncate text-[22px] leading-[1.15] font-medium tracking-[-0.4px]">
        {value} <span className="font-landing text-sm font-normal tracking-normal text-bark">{unit}</span>
      </div>
      {hint && <div className="text-[12.5px] leading-[1.4] text-bark">{hint}</div>}
    </div>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="m-0 rounded-xl border-[1.5px] border-dashed border-stone px-5 py-10 text-center text-[15px] leading-normal text-bark">{children}</p>;
}
