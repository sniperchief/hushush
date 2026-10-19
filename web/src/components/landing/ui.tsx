import Image from "next/image";
import type { ReactNode } from "react";

export function Wordmark({ className, alt = "hushush", preload }: { className: string; alt?: string; preload?: boolean }) {
  return <Image src="/hushush-wordmark.png" alt={alt} width={600} height={131} preload={preload} className={`block w-auto object-contain object-left ${className}`} />;
}

export function Mark({ className }: { className: string }) {
  return <Image src="/hushush-mark.png" alt="" width={512} height={651} className={`w-auto flex-none ${className}`} />;
}

export function Dot({ className = "bg-amber size-1.5" }: { className?: string }) {
  return <span aria-hidden="true" className={`inline-block flex-none rounded-full ${className}`} />;
}

export function ActiveTag() {
  return (
    <span className="tag border border-linen bg-paper">
      <Dot />
      Active
    </span>
  );
}

export function ArrowRight({ size = 16, className }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className={className}>
      <path d="M5 12h14M13 6l6 6-6 6" />
    </svg>
  );
}

/** Uppercase pill above a section heading. */
export function Eyebrow({ className = "bg-honey", children }: { className?: string; children: ReactNode }) {
  return <span className={`tag eyebrow self-start ${className}`}>{children}</span>;
}

export const h2Class = "m-0 text-[clamp(34px,4.6vw,56px)] leading-[1.1] font-semi tracking-[-0.02em] text-balance sm:leading-[1.13]";
const leadBase = "m-0 text-[17px] leading-[1.3] sm:text-xl sm:leading-[1.25]";
export const leadClass = `${leadBase} text-bark`;

/** Section heading row: eyebrow + h2 on the left, lead paragraph on the right. */
export function SectionIntro({
  eyebrow,
  eyebrowClass,
  title,
  lead,
  leadTone = "text-bark",
  titleMax = "max-w-[640px]",
  leadMax = "sm:max-w-[400px]",
}: {
  eyebrow: string;
  eyebrowClass?: string;
  title: string;
  lead: ReactNode;
  leadTone?: string;
  titleMax?: string;
  leadMax?: string;
}) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-x-12 gap-y-5">
      <div className={`flex flex-col gap-5 sm:gap-4 ${titleMax}`}>
        <Eyebrow className={eyebrowClass}>{eyebrow}</Eyebrow>
        <h2 className={h2Class}>{title}</h2>
      </div>
      <p className={`${leadBase} ${leadTone} ${leadMax}`}>{lead}</p>
    </div>
  );
}

export const containerClass = "mx-auto max-w-[1200px] px-4 sm:px-[clamp(16px,4vw,32px)]";
