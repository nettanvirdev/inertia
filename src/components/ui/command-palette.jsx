import * as React from "react";
import { Search } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Kbd } from "@/components/ui/kbd";
import { Portal, useEscapeLayer } from "@/components/ui/portal";
import { useFocusTrap, useLockBodyScroll } from "@/components/ui/dialog";
import { usePresence } from "@/hooks/use-presence";

/**
 * `groups` = `[{ label, items: [{ id, label, icon, shortcut, hint, keywords }] }]`
 *
 * Ranked, not just filtered: what the item is *called* beats what it is tagged
 * with, and a scattered letter match is the last resort. Without the tiers,
 * "perm" put Computers (c-o-m-**p**-u-t-**e**-**r**-s… ) above Permissions.
 */
const TIER = {
  labelPrefix: 5000,
  wordPrefix: 4000,
  labelSubstring: 3000,
  keyword: 2000,
  subsequence: 1000,
};

function isSubsequence(q, t) {
  let i = 0;
  for (let c = 0; c < t.length && i < q.length; c += 1) {
    if (t[c] === q[i]) i += 1;
  }
  return i === q.length;
}

/** Highest tier this item reaches, or -1 for no match at all. */
function score(query, label, keywordHay) {
  if (!query) return 0;
  const q = query.toLowerCase();
  const l = String(label ?? "").toLowerCase();
  const k = String(keywordHay ?? "").toLowerCase();

  const at = l.indexOf(q);
  if (at === 0) return TIER.labelPrefix + (100 - Math.min(l.length, 100));
  // a match that starts a word reads as intentional; mid-word does not
  if (at > 0 && /[\s\-_/.:]/.test(l[at - 1])) return TIER.wordPrefix - Math.min(at, 300);
  if (at > 0) return TIER.labelSubstring - Math.min(at, 300);

  const kAt = k.indexOf(q);
  if (kAt >= 0) return TIER.keyword - Math.min(kAt, 300);

  if (isSubsequence(q, l) || isSubsequence(q, k)) {
    return TIER.subsequence - Math.min(l.length, 300);
  }
  return -1;
}

function renderIcon(icon, className) {
  if (!icon) return null;
  if (React.isValidElement(icon)) {
    return React.cloneElement(icon, { className: cn(className, icon.props.className) });
  }
  const Icon = icon;
  return <Icon className={className} />;
}

export function CommandPalette({
  open,
  onOpenChange,
  groups = [],
  onSelect,
  placeholder = "Search commands…",
  className,
}) {
  const panelRef = React.useRef(null);
  const listRef = React.useRef(null);
  const inputRef = React.useRef(null);
  const [query, setQuery] = React.useState("");
  const [activeIndex, setActiveIndex] = React.useState(0);
  const baseId = React.useId();

  const close = React.useCallback(() => onOpenChange?.(false), [onOpenChange]);

  const onTabKey = useFocusTrap(panelRef, open);
  useLockBodyScroll(open);
  useEscapeLayer(open, close);
  // Held through its exit so the panel settles back up and out. Everything
  // above keys on `open`: a closing palette has already handed focus back.
  const { mounted, state } = usePresence(open);

  React.useEffect(() => {
    if (!open) return;
    setQuery("");
    setActiveIndex(0);
    const raf = requestAnimationFrame(() => inputRef.current?.focus({ preventScroll: true }));
    return () => cancelAnimationFrame(raf);
  }, [open]);

  const filtered = React.useMemo(() => {
    const q = query.trim();
    const scored = [];
    let best = -1;

    for (const group of groups) {
      const items = [];
      for (const item of group.items ?? []) {
        const keywordHay = [item.hint, ...(item.keywords ?? [])].filter(Boolean).join(" ");
        const s = q ? score(q, item.label, keywordHay) : 0;
        if (s < 0) continue;
        if (s > best) best = s;
        items.push({ item, s });
      }
      if (items.length) scored.push({ label: group.label, items });
    }

    // Past a few characters a scattered letter match is almost always noise -
    // drop it, but only while something genuinely better is on screen.
    const dropSubsequence = q.length >= 4 && best >= TIER.keyword;

    const out = [];
    for (const group of scored) {
      const items = dropSubsequence
        ? group.items.filter((x) => x.s >= TIER.keyword)
        : group.items;
      if (!items.length) continue;
      if (q) items.sort((a, b) => b.s - a.s);
      out.push({ label: group.label, items: items.map((x) => x.item) });
    }
    return out;
  }, [groups, query]);

  const flat = React.useMemo(() => filtered.flatMap((g) => g.items), [filtered]);

  React.useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  React.useEffect(() => {
    if (!open) return;
    const el = listRef.current?.querySelector('[data-active="true"]');
    el?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, open, filtered]);

  const pick = (item) => {
    if (!item) return;
    onSelect?.(item);
    close();
  };

  const onKeyDown = (e) => {
    onTabKey(e);
    if (e.defaultPrevented) return;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        if (flat.length) setActiveIndex((i) => (i + 1) % flat.length);
        break;
      case "ArrowUp":
        e.preventDefault();
        if (flat.length) setActiveIndex((i) => (i - 1 + flat.length) % flat.length);
        break;
      case "Home":
        e.preventDefault();
        setActiveIndex(0);
        break;
      case "End":
        e.preventDefault();
        setActiveIndex(Math.max(0, flat.length - 1));
        break;
      case "Enter":
        e.preventDefault();
        pick(flat[activeIndex]);
        break;
      default:
        break;
    }
  };

  if (!mounted) return null;

  let cursor = -1;

  return (
    <Portal>
      <div className={cn("fixed inset-0 z-50", state === "closing" && "pointer-events-none")}>
        <div data-state={state} className="absolute inset-0 scrim animate-fade-in" onClick={close} />
        <div className="pointer-events-none absolute inset-0 flex justify-center px-4 pt-[15vh]">
          <div
            ref={panelRef}
            role="dialog"
            aria-modal="true"
            aria-hidden={state === "closing" || undefined}
            aria-label="Command palette"
            data-state={state}
            onKeyDown={onKeyDown}
            className={cn(
              "pointer-events-auto flex h-fit w-full max-w-xl flex-col overflow-hidden",
              "overlay-surface rounded-3xl outline-none",
              "animate-overlay-in",
              className
            )}
          >
            <div className="flex h-12 shrink-0 items-center gap-2.5 px-4">
              <Search className="size-4 shrink-0 text-muted-foreground" />
              <input
                ref={inputRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={placeholder}
                role="combobox"
                aria-expanded="true"
                aria-controls={`${baseId}-list`}
                aria-activedescendant={
                  flat[activeIndex] ? `${baseId}-opt-${flat[activeIndex].id}` : undefined
                }
                autoComplete="off"
                spellCheck={false}
                // borderless by construction: the panel is the field
                className={cn(
                  "h-full w-full bg-transparent text-sm text-foreground outline-none",
                  "placeholder:text-muted-foreground"
                )}
              />
            </div>

            <div
              ref={listRef}
              id={`${baseId}-list`}
              role="listbox"
              aria-label="Results"
              className="max-h-[380px] overflow-y-auto no-scrollbar p-1 pb-2"
            >
              {flat.length === 0 ? (
                <div className="flex flex-col items-center gap-1 px-3 py-10 text-center">
                  <div className="text-[13px] text-foreground">No results</div>
                  <div className="text-[11px] text-muted-foreground">
                    Nothing matches “{query}”.
                  </div>
                </div>
              ) : (
                filtered.map((group, gi) => (
                  <div key={group.label ?? gi} role="group" aria-label={group.label}>
                    {group.label ? (
                      <div className="px-2 py-1 text-xs font-normal text-muted-foreground">
                        {group.label}
                      </div>
                    ) : null}
                    {group.items.map((item) => {
                      cursor += 1;
                      const active = cursor === activeIndex;
                      const index = cursor;
                      return (
                        <div
                          key={item.id}
                          id={`${baseId}-opt-${item.id}`}
                          role="option"
                          aria-selected={active}
                          data-active={active ? "true" : undefined}
                          onPointerEnter={() => setActiveIndex(index)}
                          onClick={() => pick(item)}
                          className={cn(
                            "flex h-9 cursor-pointer items-center gap-2 rounded-xl px-2",
                            "text-[13px] font-normal text-foreground",
                            "transition-colors duration-150 ease-out",
                            "data-[active=true]:fill-menu",
                            "[&_svg]:size-3.5 [&_svg]:shrink-0",
                            "[&_svg:not([class*='text-'])]:text-muted-foreground",
                            "data-[active=true]:[&_svg:not([class*='text-'])]:text-foreground"
                          )}
                        >
                          {renderIcon(item.icon, "size-3.5")}
                          <span className="min-w-0 flex-1 truncate">{item.label}</span>
                          {item.hint ? (
                            <span className="shrink-0 truncate text-[11px] text-muted-foreground">
                              {item.hint}
                            </span>
                          ) : null}
                          {item.shortcut ? <Kbd>{item.shortcut}</Kbd> : null}
                        </div>
                      );
                    })}
                  </div>
                ))
              )}
            </div>
          </div>
        </div>
      </div>
    </Portal>
  );
}
