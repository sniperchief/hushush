import { Banner, FinalCta, Footer, Header, Hero } from "@/components/landing/chrome";
import { Employers, Workers } from "@/components/landing/features";
import { HowItWorks } from "@/components/landing/how";
import { Privacy } from "@/components/landing/privacy";
import { Problem } from "@/components/landing/problem";
import { ProductBand } from "@/components/landing/product";
import { currentUser } from "@/lib/session";

export const metadata = {
  title: { absolute: "hushush — private streaming payroll" },
  description: "Private streaming payroll for remote teams, freelancers, DAOs, and organizations that pay employees in ZEC.",
};

/** The homepage, for everyone. Signed-in visitors get "Go to dashboard" instead of sign-up / sign-in. */
export default async function Home() {
  const signedIn = (await currentUser()) !== null;
  return (
    <div className="landing min-h-dvh overflow-x-clip bg-cream font-landing text-[15px] leading-[1.25] tracking-[-0.15px] text-espresso sm:text-base sm:tracking-[-0.16px]">
      <Banner />
      <Header signedIn={signedIn} />
      <main>
        <Hero signedIn={signedIn} />
        <ProductBand />
        <Problem />
        <Workers />
        <Employers />
        <HowItWorks />
        <Privacy />
        <FinalCta signedIn={signedIn} />
      </main>
      <Footer />
    </div>
  );
}
