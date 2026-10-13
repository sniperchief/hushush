// Display helpers only. No money arithmetic happens here: ZEC strings are
// formatted by the payroll engine and shown as-is.

/** Unix seconds → "13 Oct 2026, 14:05 UTC" (UTC so server and client agree). */
export function formatTime(unix: number | null | undefined): string {
  if (unix == null) return "—";
  return (
    new Date(unix * 1000).toLocaleString("en-GB", {
      timeZone: "UTC",
      day: "numeric",
      month: "short",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }) + " UTC"
  );
}

export function formatClock(unix: number): string {
  return new Date(unix * 1000).toLocaleTimeString("en-GB", { timeZone: "UTC" }) + " UTC";
}

/** Trims trailing zeros from an engine ZEC string for compact display: "1.50000000" → "1.5". */
export function compactZec(zec: string): string {
  return zec.includes(".") ? zec.replace(/0+$/, "").replace(/\.$/, "") : zec;
}

export function roleLabel(role: string): string {
  return role === "member" ? "Contributor" : role === "admin" ? "Admin" : "Owner";
}
