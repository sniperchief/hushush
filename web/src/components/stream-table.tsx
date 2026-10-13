import Link from "next/link";
import { compactZec } from "@/lib/format";
import type { StreamView } from "@/lib/payroll";
import { periodLabel } from "@/lib/periods";
import { StreamStatusBadge } from "./status";

export function StreamTable({ streams, showRecipient, showOrg }: { streams: StreamView[]; showRecipient?: boolean; showOrg?: boolean }) {
  return (
    <div className="-mx-5 overflow-x-auto sm:-mx-6">
      <table className="w-full min-w-[720px] text-sm">
        <thead>
          <tr className="border-b border-line text-left text-xs uppercase tracking-wide text-ink-soft">
            <th className="px-5 py-2 font-medium sm:px-6">{showRecipient ? "Contributor" : showOrg ? "Organization" : "Stream"}</th>
            <th className="px-3 py-2 font-medium">Rate</th>
            <th className="px-3 py-2 font-medium">Status</th>
            <th className="px-3 py-2 text-right font-medium">Earned</th>
            <th className="px-3 py-2 text-right font-medium">Reserved</th>
            <th className="px-3 py-2 text-right font-medium">Claimed</th>
            <th className="px-5 py-2 text-right font-medium sm:px-6">Available</th>
          </tr>
        </thead>
        <tbody>
          {streams.map((s) => (
            <tr key={s.id} className="border-b border-line last:border-0 hover:bg-muted/60">
              <td className="px-5 py-3 sm:px-6">
                <Link href={`/app/streams/${s.id}`} className="font-medium hover:underline">
                  {showRecipient ? s.recipient_display_name : s.organization_name}
                </Link>
                {showRecipient && <div className="text-xs text-ink-soft">{s.recipient_email}</div>}
              </td>
              <td className="whitespace-nowrap px-3 py-3 font-mono tabular-nums">
                {compactZec(s.rate_zec)} <span className="font-sans text-ink-soft">ZEC {periodLabel(s.rate_period_secs)}</span>
              </td>
              <td className="px-3 py-3"><StreamStatusBadge status={s.status} /></td>
              <td className="px-3 py-3 text-right font-mono tabular-nums">{s.balance.earned_zec}</td>
              <td className="px-3 py-3 text-right font-mono tabular-nums">{s.balance.reserved_zec}</td>
              <td className="px-3 py-3 text-right font-mono tabular-nums">{s.balance.claimed_zec}</td>
              <td className="px-5 py-3 text-right font-mono font-semibold tabular-nums sm:px-6">{s.balance.available_zec}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
