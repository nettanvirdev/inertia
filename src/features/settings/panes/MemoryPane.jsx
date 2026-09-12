import * as React from "react";
import { useApp } from "@/lib/store";
import { PREFERENCE_DEFAULTS } from "@/lib/appearance";
import { mcp } from "@/lib/integrations";
import { DEFAULT_INSTRUCTIONS, INJECT_BUDGET_BYTES, MEMORY_CAPTURE } from "@shared/memory";
import { PROJECT_INIT } from "@shared/project-init";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { Select } from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { SettingsRow, SettingsSection } from "../SettingsRow";

/**
 * Everything about what the app remembers.
 *
 * The pane is arranged the way the feature is understood rather than the way it
 * is built: whether to remember at all, then what gets written down, then where
 * it is kept, then the project files. A person who reads only the first switch
 * and leaves should still get the behaviour they expect.
 *
 * The one thing this screen must never do is lie. Every control here changes
 * what the main process actually does on the next turn - the tools an agent
 * holds, the block in its prompt, the navigation in the sidebar - rather than
 * hiding something that keeps running underneath.
 */

/** Roughly what a byte budget is worth in memories, for a label a person can read. */
function budgetLabel(bytes) {
  const entries = Math.max(1, Math.round(bytes / 128));
  return `about ${entries} ${entries === 1 ? "memory" : "memories"}`;
}

export function MemoryPane() {
  const { user, setPreference } = useApp();
  const prefs = user?.preferences ?? {};

  const enabled = prefs.memory !== false;
  const backend = prefs.memoryBackend ?? "local";
  const instructions = prefs.memoryInstructions ?? "";
  const budget = Number(prefs.memoryBudget) || INJECT_BUDGET_BYTES;

  /**
   * The connected MCP servers, offered as places to keep memories.
   *
   * Read once when the pane opens rather than subscribed to: a person who
   * connects a server while this screen is open is on the Integrations screen,
   * and will come back to a fresh list. Failing to read them leaves the
   * built-in option, which is the one that always works.
   */
  const [servers, setServers] = React.useState([]);
  React.useEffect(() => {
    let alive = true;
    mcp
      .list()
      .then((rows) => alive && setServers(rows ?? []))
      .catch(() => alive && setServers([]));
    return () => {
      alive = false;
    };
  }, []);

  const backendOptions = [
    {
      value: "local",
      label: "This workspace",
      description: "Memories are files in your workspace folder. Nothing leaves this machine.",
    },
    ...servers
      .filter((server) => server.enabled !== false)
      .map((server) => ({
        value: `mcp:${server.id}`,
        label: server.name || server.id,
        description:
          server.status === "connected"
            ? "A connected memory server. Shared with any other agent pointed at it."
            : "Not connected right now. Memories fall back to this workspace until it is.",
      })),
  ];

  return (
    <>
      <SettingsSection
        title="Memory"
        description="What Inertia remembers between conversations, so you do not have to explain the same things twice."
      >
        <SettingsRow
          label="Remember things between conversations"
          htmlFor="set-memory"
          description="When this is off, nothing is written down, nothing already saved is used, and the Memory screen is hidden."
          control={
            <Switch
              id="set-memory"
              size="sm"
              label="Remember things between conversations"
              checked={enabled}
              onCheckedChange={(v) => setPreference("memory", v)}
            />
          }
        />
        {/* The rows fold under the switch that governs them. The inner box
            carries the card's hairlines so the fold reads as more of the
            same list rather than a second one. */}
        <Collapse open={enabled} innerClassName="divide-border-subtle divide-y">
            <SettingsRow
              label="Remember things about each project"
              htmlFor="set-memory-project"
              description="Conventions, decisions and unfinished work, kept per folder. A project's memories are never used in another project."
              control={
                <Switch
                  id="set-memory-project"
                  size="sm"
                  label="Remember things about each project"
                  checked={prefs.memoryProject !== false}
                  onCheckedChange={(v) => setPreference("memoryProject", v)}
                />
              }
            />
            <SettingsRow
              label="When to write things down"
              description="Looking for something worth keeping costs one quick call to your model."
              control={
                <Select
                  size="xs"
                  className="w-56"
                  ariaLabel="When to write things down"
                  value={prefs.memoryCapture ?? PREFERENCE_DEFAULTS.memoryCapture}
                  onChange={(v) => setPreference("memoryCapture", v)}
                  options={MEMORY_CAPTURE.map((mode) => ({
                    value: mode.id,
                    label: mode.label,
                    description: mode.hint,
                  }))}
                />
              }
            />
            <SettingsRow
              label="Ask before saving"
              htmlFor="set-memory-review"
              description="New memories wait on the Memory screen for you to approve them instead of being saved straight away."
              control={
                <Switch
                  id="set-memory-review"
                  size="sm"
                  label="Ask before saving"
                  checked={prefs.memoryReview === true}
                  onCheckedChange={(v) => setPreference("memoryReview", v)}
                />
              }
            />
            <SettingsRow
              label="How much to carry into each conversation"
              description={`Titles only, ${budgetLabel(budget)}. The rest are still found when they are searched for.`}
              control={
                <Slider
                  className="w-40"
                  label="How much to carry into each conversation"
                  min={512}
                  max={8192}
                  step={256}
                  value={budget}
                  onChange={(v) => setPreference("memoryBudget", v)}
                  formatValue={(v) => budgetLabel(v)}
                />
              }
            />
        </Collapse>
      </SettingsSection>

      <Collapse open={enabled} className="mt-7">
          <SettingsSection
            title="What to remember"
            description="The instructions followed when deciding what is worth writing down. The default is written to work well; change it if you want something different kept."
          >
            <div className="px-4 py-3">
              <Textarea
                rows={12}
                spellCheck={false}
                aria-label="What to remember"
                className="font-mono text-[12px] leading-relaxed"
                placeholder={DEFAULT_INSTRUCTIONS}
                value={instructions}
                onChange={(e) => setPreference("memoryInstructions", e.target.value)}
              />
              <div className="mt-2 flex items-center justify-between gap-3">
                <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
                  {instructions.trim()
                    ? "Your own instructions are in use."
                    : "Empty, so the default shown above is used."}
                </p>
                <Button
                  size="xs"
                  variant="ghost"
                  disabled={!instructions.trim()}
                  onClick={() => setPreference("memoryInstructions", "")}
                >
                  Restore the default
                </Button>
              </div>
            </div>
          </SettingsSection>

          <SettingsSection
            title="Where memories are kept"
            description="By default, files on this machine: what is about you lives in your workspace folder, and what is about a project lives in that project's own .inertia/memory/, so it travels with the repository. Point this at a memory server and the same memories are shared with any other coding agent connected to it."
          >
            <SettingsRow
              label="Storage"
              description={
                backend === "local"
                  ? "Nothing leaves this machine."
                  : "Memories are sent to and read from this server."
              }
              control={
                <Select
                  size="xs"
                  className="w-56"
                  ariaLabel="Storage"
                  value={backend}
                  onChange={(v) => setPreference("memoryBackend", v)}
                  options={backendOptions}
                />
              }
            />
            <Collapse open={backend !== "local"}>
              <SettingsRow
                label="If the server cannot be reached"
                description="Conversations carry on using this workspace's own memories, and a note explains why. A memory server being down never stops you working."
              />
            </Collapse>
          </SettingsSection>
      </Collapse>

      <SettingsSection
        title="Project instructions"
        description="AGENTS.md and .inertia/rules/ are read at the start of every conversation, from the top of the repository down to the folder you are in. Every agent that reads AGENTS.md benefits from the same file, not only Inertia."
      >
        <SettingsRow
          label="Setting up a new project"
          description="Nothing is ever written into a repository without asking first."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Setting up a new project"
              value={prefs.projectInit ?? PREFERENCE_DEFAULTS.projectInit}
              onChange={(v) => setPreference("projectInit", v)}
              options={PROJECT_INIT.map((mode) => ({
                value: mode.id,
                label: mode.label,
                description: mode.hint,
              }))}
            />
          }
        />
      </SettingsSection>
    </>
  );
}
