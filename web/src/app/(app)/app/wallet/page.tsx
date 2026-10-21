import { redirect } from "next/navigation";
import { CopyButton } from "@/components/dashboard/copy-button";
import { FaucetButton } from "@/components/dashboard/faucet-button";
import { Card, PageHeader, sumOf } from "@/components/dashboard/sections";
import { AutoRefresh } from "@/components/worker";
import { formatZats } from "@/lib/chart-data";
import { companyOf, employerHomeOf, payoutWalletOf } from "@/lib/dashboard";
import { compactZec, formatTime } from "@/lib/format";
import { requirePageUser } from "@/lib/session";

export const metadata = { title: "Payout wallet" };
export const dynamic = "force-dynamic";

/** The company's own account in the testnet payout wallet (employers). */
export default async function WalletPage() {
  const user = await requirePageUser("/app/wallet");
  const company = user.intent === "pay" ? await companyOf(user.id) : null;
  if (!company) redirect("/app");
  const [wallet, home] = await Promise.all([payoutWalletOf(user.id, company.id), employerHomeOf(user.id, company.id)]);
  const owedNow = sumOf(home.streams, (s) => s.balance.available_zats);
  const stillToEarn = sumOf(home.streams, (s) => s.total_zats - s.balance.earned_zats);

  return (
    <>
      <AutoRefresh seconds={30} />
      <PageHeader
        title="Payout wallet"
        lead={`${company.name}'s own account in the hushush payout wallet. Every withdrawal, and its network fee, is paid from it. Zcash testnet: TAZ has no monetary value.`}
      />

      {typeof wallet === "string" ? (
        <Card title="Unavailable" icon="treasury">
          <p className="m-0 text-sm leading-normal text-bark">{wallet}</p>
        </Card>
      ) : (
        <div className="grid items-start gap-4 min-[1180px]:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
          <section aria-labelledby="wallet-balance" className="flex min-w-0 flex-col gap-4 rounded-xl border border-espresso bg-espresso px-5 py-[22px] text-cream sm:px-6">
            <div className="flex items-center gap-2 text-[13px] text-oat">
              <h2 id="wallet-balance" className="m-0 text-[13px] font-normal">
                Spendable now
              </h2>
              <span className="rounded-full bg-honey px-[7px] py-px text-[11px] font-semi text-espresso">Testnet</span>
            </div>
            <span className="mono text-[36px] leading-[1.05] font-medium tracking-[-0.8px] break-all text-amber">
              {compactZec(wallet.spendable_zec)} <span className="font-landing text-base font-normal tracking-normal text-oat">TAZ</span>
            </span>
            {wallet.pending_zats > 0 && <span className="text-[13px] text-oat">+{compactZec(wallet.pending_zec)} TAZ settling (spendable after a few confirmations)</span>}
            <div className="flex flex-col gap-1.5">
              <span className="text-[13px] text-oat">Funding address</span>
              <div className="flex items-center gap-2 rounded-lg border border-cream/14 bg-cream/8 py-1.5 pr-1.5 pl-3">
                <span className="mono min-w-0 flex-1 text-[12px] break-all">{wallet.funding_address}</span>
                <CopyButton text={wallet.funding_address} label="Copy funding address" />
              </div>
              <span className="text-[12.5px] leading-[1.45] text-oat">Send testnet TAZ to this shielded address to fund withdrawals.</span>
            </div>
            <FaucetButton
              available={wallet.faucet.available}
              amountZec={compactZec(wallet.faucet.amount_zec)}
              nextAt={wallet.faucet.next_at ? formatTime(wallet.faucet.next_at) : null}
              remaining={wallet.faucet.remaining}
            />
          </section>

          <Card title="Can it cover your streams?" icon="treasury">
            <dl className="m-0 grid grid-cols-[1fr_auto] gap-x-4 gap-y-3 text-sm">
              <dt className="text-bark">Workers can withdraw now</dt>
              <dd className="mono m-0 text-right">{formatZats(owedNow)} ZEC</dd>
              <dt className="text-bark">Still to be earned</dt>
              <dd className="mono m-0 text-right">{formatZats(stillToEarn)} ZEC</dd>
              <dt className="text-bark">Spendable in this wallet</dt>
              <dd className="mono m-0 text-right font-semi">{compactZec(wallet.spendable_zec)} TAZ</dd>
            </dl>
            <p className={`m-0 rounded-lg border px-3.5 py-3 text-[13.5px] leading-[1.45] ${owedNow > BigInt(wallet.spendable_zats) ? "border-honey bg-honey-soft" : "border-linen bg-cream text-bark"}`}>
              {owedNow > BigInt(wallet.spendable_zats)
                ? "The wallet can't cover what workers can withdraw right now. Fund it so their withdrawals don't wait."
                : "The wallet covers what workers can withdraw right now. Keep it funded as they earn more."}
            </p>
            <p className="m-0 text-[12.5px] leading-[1.45] text-bark">
              Withdrawals are confirmed after {wallet.required_confirmations} blocks.
              {wallet.last_sync_at && ` Last synced with the network ${formatTime(wallet.last_sync_at)}.`}
            </p>
          </Card>
        </div>
      )}
    </>
  );
}
