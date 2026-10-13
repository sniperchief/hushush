import type { ComponentProps, ReactNode } from "react";

export function cx(...c: (string | false | null | undefined)[]): string {
  return c.filter(Boolean).join(" ");
}

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  return <section className={cx("rounded-2xl border border-line bg-surface p-5 sm:p-6 shadow-sm", className)}>{children}</section>;
}

export function CardTitle({ children, action }: { children: ReactNode; action?: ReactNode }) {
  return (
    <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
      <h2 className="text-base font-semibold tracking-tight">{children}</h2>
      {action}
    </div>
  );
}

const tones = {
  neutral: "bg-muted text-ink-soft",
  green: "bg-ok-bg text-ok",
  amber: "bg-warn-bg text-warn",
  red: "bg-bad-bg text-bad",
  accent: "bg-accent-bg text-accent-ink",
} as const;

export function Badge({ children, tone = "neutral", title }: { children: ReactNode; tone?: keyof typeof tones; title?: string }) {
  return (
    <span title={title} className={cx("inline-flex items-center gap-1 whitespace-nowrap rounded-full px-2.5 py-0.5 text-xs font-medium", tones[tone])}>
      {children}
    </span>
  );
}

export function Button({ variant = "primary", className, ...props }: ComponentProps<"button"> & { variant?: "primary" | "secondary" | "danger" | "ghost" }) {
  const styles = {
    primary: "bg-ink text-surface hover:opacity-90",
    secondary: "border border-line bg-surface text-ink hover:bg-muted",
    danger: "border border-bad/30 bg-bad-bg text-bad hover:border-bad/60",
    ghost: "text-ink-soft hover:bg-muted hover:text-ink",
  }[variant];
  return (
    <button
      {...props}
      className={cx(
        "inline-flex min-h-10 items-center justify-center gap-2 rounded-xl px-4 text-sm font-medium transition disabled:cursor-not-allowed disabled:opacity-50",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        styles,
        className,
      )}
    />
  );
}

export function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-sm font-medium">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-xs text-ink-soft">{hint}</span>}
    </label>
  );
}

export const inputClass =
  "block w-full min-h-10 rounded-xl border border-line bg-surface px-3 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none focus:ring-2 focus:ring-accent/25";

export function Alert({ tone = "red", children }: { tone?: "red" | "green" | "amber" | "neutral"; children: ReactNode }) {
  const t = { red: "border-bad/30 bg-bad-bg text-bad", green: "border-ok/30 bg-ok-bg text-ok", amber: "border-warn/30 bg-warn-bg text-warn", neutral: "border-line bg-muted text-ink-soft" }[tone];
  return (
    <div role={tone === "red" ? "alert" : "status"} className={cx("rounded-xl border px-3.5 py-2.5 text-sm", t)}>
      {children}
    </div>
  );
}

export function Stat({ label, value, unit = "ZEC", hint, emphasis }: { label: string; value: string; unit?: string; hint?: string; emphasis?: boolean }) {
  return (
    <div className={cx("rounded-xl border border-line p-4", emphasis ? "bg-accent-bg" : "bg-surface")}>
      <div className="text-xs font-medium uppercase tracking-wide text-ink-soft">{label}</div>
      <div className="mt-1 font-mono text-lg font-semibold tabular-nums sm:text-xl">
        {value} <span className="text-sm font-normal text-ink-soft">{unit}</span>
      </div>
      {hint && <div className="mt-1 text-xs text-ink-soft">{hint}</div>}
    </div>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="rounded-xl border border-dashed border-line px-4 py-8 text-center text-sm text-ink-soft">{children}</p>;
}
