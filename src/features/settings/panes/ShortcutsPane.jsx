import * as React from "react";
import { Keyboard } from "@/components/icons";
import { getShortcutGroups } from "@/data/shortcuts";
import { useApp } from "@/lib/store";
import { SearchInput } from "@/components/ui/input";
import { Kbd, formatShortcut } from "@/components/ui/kbd";
import { SettingsCard, SettingsSection } from "../SettingsRow";

export function ShortcutsPane() {
  const { user } = useApp();
  const [query, setQuery] = React.useState("");
  const sendOnEnter = user.preferences?.sendOnEnter !== false;

  const groups = React.useMemo(() => {
    const all = getShortcutGroups({ sendOnEnter });
    const q = query.trim().toLowerCase();
    if (!q) return all;
    return all
      .map((group) => ({
        ...group,
        // The rendered chip as well as the raw combo, because someone typing
        // "ctrl" is reading the screen, not the "mod+k" the handler is bound
        // under.
        shortcuts: group.shortcuts.filter(
          (s) =>
            s.label.toLowerCase().includes(q) ||
            s.combo.toLowerCase().includes(q) ||
            formatShortcut(s.combo).toLowerCase().includes(q) ||
            group.name.toLowerCase().includes(q)
        ),
      }))
      .filter((group) => group.shortcuts.length > 0);
  }, [query, sendOnEnter]);

  return (
    <div className="w-full">
      <div className="flex items-center gap-2">
        <SearchInput
          size="xs"
          className="flex-1"
          placeholder="Search shortcuts"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onClear={() => setQuery("")}
        />
      </div>

      {groups.length === 0 ? (
        // in-band empty state - EmptyState paints at 16/13px and the sheet is 12px
        <div className="flex flex-col items-center gap-2 px-6 py-12 text-center">
          <span className="flex size-8 items-center justify-center rounded-full fill-control text-muted-foreground">
            <Keyboard className="size-4" aria-hidden="true" />
          </span>
          <p className="text-xs text-foreground/90">No shortcut matches</p>
          <p className="max-w-72 text-[0.6875rem] leading-relaxed text-muted-foreground">
            Nothing in the reference matches “{query}”. Try a key name like Ctrl, or an action
            like chat.
          </p>
        </div>
      ) : (
        // one full-width column: action on the left, its chips on the right edge.
        // The wrapper is what `first:mt-0` measures against, so the first group
        // sits flush under the search row and the rest keep the 20px rhythm.
        <div className="mt-5">
          {groups.map((group) => (
            <SettingsSection flat key={group.name} title={group.name}>
              <SettingsCard className="flex flex-col gap-0.5 py-1.5">
                {group.shortcuts.map((shortcut) => (
                  <div
                    key={shortcut.id}
                    className="flex items-center justify-between gap-4 rounded-md px-1 py-1"
                  >
                    <span className="min-w-0 truncate text-xs text-foreground/90">
                      {shortcut.label}
                    </span>
                    <Kbd className="shrink-0">{formatShortcut(shortcut.combo)}</Kbd>
                  </div>
                ))}
              </SettingsCard>
            </SettingsSection>
          ))}
        </div>
      )}
    </div>
  );
}
