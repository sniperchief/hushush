/** Pay periods offered in the UI. Months are fixed 30-day periods. */
export const RATE_PERIODS = [
  { secs: 3600, label: "per hour" },
  { secs: 86400, label: "per day" },
  { secs: 604800, label: "per week" },
  { secs: 2592000, label: "per 30 days" },
  { secs: 31536000, label: "per 365 days" },
] as const;

export function periodLabel(secs: number): string {
  return RATE_PERIODS.find((p) => p.secs === secs)?.label ?? `per ${secs.toLocaleString("en-US")} s`;
}
