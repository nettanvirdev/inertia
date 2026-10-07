import * as React from "react";
import { FolderOpen, LayoutGrid, Plus, Rows3 } from "@/components/icons";
import { useWorkspace } from "@/lib/workspace";
import { PREF, usePersistentState } from "@/lib/persist";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/input";
import { Tabs } from "@/components/ui/tabs";
import { Tooltip } from "@/components/ui/tooltip";
import { Segmented } from "@/components/ui/segmented";
import { IconButton } from "@/components/ui/icon-button";
import { EmptyState } from "@/components/ui/empty-state";
import { PLUGIN_KINDS, PLUGIN_KIND_IDS, getPluginKind } from "./kinds";
import { PluginManager } from "./PluginManager";
import { GUTTER, HEADER_GAP } from "@/components/layout/View";
import { cn } from "@/lib/utils";

/**
 * Everything an agent can call, grouped by where it comes from.
 *
 * The four tabs are four folders in the workspace, and each one is a full
 * manager rather than a catalogue: what is on this screen is what is on disk,
 * and adding, editing or removing a row writes a file. The tabs used to filter
 * a demo catalogue by category; a category is not a thing a user manages, so
 * they were replaced outright.
 */

const KIND_IDS = new Set(PLUGIN_KIND_IDS);

export function IntegrationsView() {
  const { configured, native, reveal } = useWorkspace();

  const [query, setQuery] = React.useState("");
  // The tab is a preference; the search box deliberately is not - a remembered
  // query would reopen the page filtered with no obvious reason why. The
  // validator rejects the old category values, so an upgrading user who last
  // left this screen on "Storage" lands on Composio rather than on nothing.
  const [tab, setTab] = usePersistentState(PREF.integrationsTab, "composio", (v) =>
    KIND_IDS.has(v)
  );
  const [layout, setLayout] = usePersistentState(
    PREF.integrationsLayout,
    "rows",
    (v) => v === "rows" || v === "grid"
  );
  const [addToken, setAddToken] = React.useState(0);

  const kind = getPluginKind(tab);
  const folder = kind.collection.replace(".", "/");

  const tabs = React.useMemo(
    () => PLUGIN_KINDS.map((k) => ({ value: k.id, label: k.label, icon: k.icon })),
    []
  );

  return (
    <>
      <header className={cn("flex h-14 shrink-0 items-center", HEADER_GAP, GUTTER)}>
        <h1 className="text-sm font-medium text-foreground">Integrations</h1>
        <p className="hidden shrink-0 text-[11px] text-muted-foreground sm:block">
          Everything your agents can call
        </p>

        <div className="ml-auto flex items-center gap-2">
          <SearchInput
            size="sm"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onClear={() => setQuery("")}
            placeholder={kind.searchPlaceholder}
            className="w-44 lg:w-56"
            aria-label={kind.searchPlaceholder}
          />
          <Segmented
            value={layout}
            onChange={setLayout}
            label="Layout"
            options={[
              { value: "rows", icon: <Rows3 />, label: null },
              { value: "grid", icon: <LayoutGrid />, label: null },
            ]}
          />
          {native && configured ? (
            <Tooltip content={`Open ${folder} in the file explorer`}>
              <IconButton size="lg" label={`Open ${folder}`} onClick={() => reveal(folder)}>
                <FolderOpen />
              </IconButton>
            </Tooltip>
          ) : null}
          {/* Composio has no "add" - an app is connected by picking it out of
              the catalogue further down the page, and a button that only
              scrolled somewhere would be a lie about what it does. */}
          {kind.hideAdd ? null : (
            <Button
              variant="primary"
              size="sm"
              disabled={!configured}
              onClick={() => setAddToken((n) => n + 1)}
            >
              <Plus />
              {kind.addLabel}
            </Button>
          )}
        </div>
      </header>

      <div className={cn("pb-1", GUTTER)}>
        <Tabs
          variant="pill"
          value={tab}
          onChange={setTab}
          items={tabs}
          ariaLabel="Plugin kind"
          idPrefix="int-kind"
        />
      </div>

      {configured ? (
        // Keyed by tab so a switch starts a manager clean: no half-typed draft
        // from the previous kind, and no list from the previous folder.
        //
        // A kind that owns a live backend brings its own screen, because a row
        // in a folder and a running MCP server are not the same object: one has
        // a status, tools and a reason it failed, and none of that fits the
        // generic list/edit machinery. The kinds that are only records - skills
        // - still get it for free.
        kind.Screen ? (
          <kind.Screen key={kind.id} query={query} layout={layout} addToken={addToken} />
        ) : (
          <PluginManager
            key={kind.id}
            kind={kind}
            query={query}
            layout={layout}
            addToken={addToken}
          />
        )
      ) : (
        <div className={cn("min-h-0 flex-1 pt-2", GUTTER)}>
          <EmptyState
            icon={FolderOpen}
            title="No workspace folder yet"
            description="Plugins and skills live in the workspace folder, so there is nothing to manage until one is chosen. Pick a folder in Settings and this screen fills itself in."
          />
        </div>
      )}
    </>
  );
}
