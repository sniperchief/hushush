import { initials } from "@/lib/chart-data";

const TONES = ["bg-wisteria text-espresso", "bg-honey text-espresso", "bg-linen text-espresso", "bg-mist text-bark"];

/** A stable avatar colour per name. */
export function avatarTone(name: string): string {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return TONES[h % TONES.length]!;
}

export function Avatar({ name, size = 30, dashed = false }: { name: string; size?: 26 | 30; dashed?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={`inline-flex flex-none items-center justify-center rounded-full font-semi ${size === 26 ? "size-[26px] text-[10px]" : "size-[30px] text-[11px]"} ${
        dashed ? "border-[1.5px] border-dashed border-stone bg-paper text-bark" : avatarTone(name)
      }`}
    >
      {initials(name)}
    </span>
  );
}
