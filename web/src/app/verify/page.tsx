import { VerifyEmail } from "@/components/forms/auth-forms";
import { Card } from "@/components/ui";

export const metadata = { title: "Verify email" };

type Props = { searchParams: Promise<Record<string, string | string[] | undefined>> };

// The link from the email lands here; verification needs an explicit click so
// mail scanners that prefetch links can't consume the single-use token.
export default async function VerifyPage({ searchParams }: Props) {
  const sp = await searchParams;
  const token = typeof sp.token === "string" ? sp.token : "";
  return (
    <div className="mx-auto max-w-md">
      <h1 className="mb-6 text-2xl font-semibold tracking-tight">Verify your email</h1>
      <Card>
        <VerifyEmail token={token} />
      </Card>
    </div>
  );
}
