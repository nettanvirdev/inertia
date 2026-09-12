import * as React from "react";
import { cn } from "@/lib/utils";

function isMac() {
  if (typeof navigator === "undefined") return false;
  const src = navigator.userAgentData?.platform || navigator.platform || navigator.userAgent || "";
  return /mac|iphone|ipad|ipod/i.test(src);
}

const MAC = { mod: "⌘", meta: "⌘", cmd: "⌘", alt: "⌥", option: "⌥", shift: "⇧", ctrl: "⌃", control: "⌃" };
const PC = { mod: "Ctrl", meta: "Win", cmd: "Ctrl", alt: "Alt", option: "Alt", shift: "Shift", ctrl: "Ctrl", control: "Ctrl" };

const NAMED = {
  enter: "↵",
  return: "↵",
  escape: "Esc",
  esc: "Esc",
  backspace: "⌫",
  delete: "Del",
  tab: "⇥",
  space: "Space",
  up: "↑",
  down: "↓",
  left: "←",
  right: "→",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
};

/** "mod+k" → "⌘K" on macOS, "Ctrl K" elsewhere. */
function formatShortcut(combo) {
  if (!combo) return "";
  const mac = isMac();
  const table = mac ? MAC : PC;
  const parts = String(combo)
    .split("+")
    .map((p) => p.trim().toLowerCase())
    .filter(Boolean)
    .map((p) => table[p] ?? NAMED[p] ?? (p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)));
  // mac glyphs read as one unit; word modifiers need the space
  return mac ? parts.join("") : parts.join(" ");
}

function Kbd({ children, className, ...props }) {
  return (
    <kbd
      className={cn(
        "inline-flex h-5 shrink-0 items-center justify-center rounded-[6px] px-1.5",
        "fill-secondary font-sans text-[10px] leading-none text-muted-foreground/70",
        className
      )}
      {...props}
    >
      {children}
    </kbd>
  );
}

export { Kbd, formatShortcut };
