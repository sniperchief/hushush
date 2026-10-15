"use client";

import { useState } from "react";
import { Icon } from "./icons";

/** Icon button that copies `text`, showing a tick for 2 seconds. */
export function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      aria-label={copied ? "Copied" : label}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text);
          setCopied(true);
          setTimeout(() => setCopied(false), 2000);
        } catch {
          /* clipboard blocked: the address stays selectable */
        }
      }}
      className="flex size-7 flex-none cursor-pointer items-center justify-center rounded-md bg-cream/10 text-cream hover:bg-cream/20"
    >
      <Icon name={copied ? "check" : "copy"} size={15} />
    </button>
  );
}
