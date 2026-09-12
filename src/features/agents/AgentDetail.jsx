import * as React from "react";
import { ArrowLeft, Copy, MessageSquare, MoreHorizontal, Pause, Play, Plus, Search, Settings2, ShieldAlert, ShieldCheck, Trash2, Icon } from "@/components/icons";
import {
  AGENT_STATUS_META,
  formatCompactNumber,
  relativeTime,
} from "@/data";
import { useApp } from "@/lib/store";
import { ANY, asRules, evaluate, merge } from "@shared/permission";
import { groupedTools } from "@shared/tools";
import { isProtected } from "@shared/agents";
import { useToast } from "@/components/ui/toast";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { DropdownMenu, MenuItem, MenuSeparator } from "@/components/ui/dropdown-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { IconButton } from "@/components/ui/icon-button";
import { SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select } from "@/components/ui/select";
import { Tabs, TabPanel } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { PermissionMatrix } from "@/features/agents/PermissionMatrix";
import { AgentEditorDialog } from "@/features/agents/AgentEditorDialog";
import { MemoryCard } from "@/features/memory/MemoryCard";
import { NewMemoryDialog } from "@/features/memory/NewMemoryDialog";
import { RoutineRow } from "@/features/routines/RoutineRow";
import { ActivityRow } from "@/features/activity/ActivityRow";

const STATUS_VARIANT = { online: "success", busy: "warning", idle: "neutral", offline: "neutral" };

const TABS = [
  { value: "overview", label: "Overview", icon: "Info" },
  { value: "routines", label: "Routines", icon: "Repeat" },
  { value: "memory", label: "Memory", icon: "Brain" },
  { value: "permissions", label: "Permissions", icon: "ShieldCheck" },
  { value: "activity", label: "Activity", icon: "Activity" },
];

const ACTIVITY_CAP = 8;

/**
 * The models this workspace can actually send to.
 *
 * Read from the configured providers, not from the shipped catalogue. This used
 * to list Claude, GPT, Gemini, Ollama and OpenRouter to everyone, because it
 * was built against the demo fixtures and nobody moved it when the real
 * provider settings arrived. A picker offering models the workspace has no key
 * for is not a picker; every choice in it fails at the first turn, and the one
 * model the user did configure was not in the list.
 *
 * "Workspace default" is first and is what an agent gets when it has chosen
 * nothing, so the empty state is a real option rather than a blank.
 */
function modelOptions(chatModels, defaultLabel) {
  const groups = new Map();
  for (const model of chatModels ?? []) {
    const name = model.providerName || "Configured";
    if (!groups.has(name)) groups.set(name, []);
    groups.get(name).push({ value: model.ref, label: model.label, description: model.id });
  }
  return [
    {
      group: "Default",
      options: [
        {
          value: "",
          label: "Workspace default",
          description: defaultLabel || "Set under Settings, Providers",
        },
      ],
    },
    ...[...groups.entries()].map(([name, options]) => ({ group: name, options })),
  ];
}

/**
 * How long this agent has been on the team.
 *
 * This slot used to read "Uptime", off a `uptimeDays` field written once when
 * the agent was created and never touched again - so it said the same number
 * on day one and day ninety. An agent is not a server anyway: it is not "up",
 * it exists. Derived from the date it was made, which is a fact the record
 * already holds and cannot go stale.
 */
function ageOf(agent) {
  const born = Date.parse(agent?.createdAt ?? "");
  if (!Number.isFinite(born)) return "-";
  const days = Math.max(0, Math.floor((Date.now() - born) / 86_400_000));
  if (days < 1) return "today";
  if (days < 30) return `${days}d`;
  if (days < 365) return `${Math.floor(days / 30)}mo`;
  return `${Math.floor(days / 365)}y`;
}

function Stat({ label, value }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className="text-lg leading-none font-medium tabular-nums text-foreground">{value}</span>
      <span className="text-[11px] text-muted-foreground">{label}</span>
    </div>
  );
}

export function AgentDetail({ agentId, onBack }) {
  const {
    agents,
    threads,
    routines,
    memories,
    activity,
    computers,
    createThread,
    openThread,
    runRoutine,
    saveAgent,
    deleteAgent,
    createAgent,
    pinMemory,
    setView,
    permissions,
    chatModels,
    defaultModelRef,
  } = useApp();
  const { toast } = useToast();

  const agent = agents.find((b) => b.id === agentId);

  const models = React.useMemo(() => {
    const fallback = chatModels?.find((m) => m.ref === defaultModelRef);
    return modelOptions(chatModels, fallback ? `Currently ${fallback.label}` : "");
  }, [chatModels, defaultModelRef]);

  const [tab, setTab] = React.useState("overview");

  // Counted off the merged rules rather than a field on the agent, because the
  // rules are what the tool registry filters on when a turn is built.
  const { totalTools, allowedCount } = React.useMemo(() => {
    const rows = groupedTools().flatMap((group) => group.tools);
    const merged = merge(asRules(permissions?.workspace), asRules(permissions?.agents?.[agent?.id]));
    const allowed = rows.filter((tool) => {
      const verdict = evaluate(merged, tool.key, ANY);
      return !(verdict.action === "deny" && (verdict.rule?.pattern ?? ANY) === ANY);
    });
    return { totalTools: rows.length, allowedCount: allowed.length };
  }, [permissions, agent?.id]);
  const [prompt, setPrompt] = React.useState(agent?.systemPrompt ?? "");
  const [memoryQuery, setMemoryQuery] = React.useState("");
  const [editorOpen, setEditorOpen] = React.useState(false);
  const [confirmDelete, setConfirmDelete] = React.useState(false);
  const [addingMemory, setAddingMemory] = React.useState(false);

  React.useEffect(() => {
    setPrompt(agent?.systemPrompt ?? "");
    setTab("overview");
    setMemoryQuery("");
    setAddingMemory(false);
  }, [agentId]); // eslint-disable-line react-hooks/exhaustive-deps

  /**
   * This agent's memories, filtered and ordered.
   *
   * Above the "agent not found" return rather than beside the list it feeds,
   * because a hook after an early return runs on some renders and not others.
   * It also now depends on `memories` and the id rather than on an array
   * rebuilt every render, which is what the dependency was actually watching
   * before.
   */
  const agentMemories = React.useMemo(
    () => memories.filter((m) => m.agentId === agent?.id),
    [memories, agent?.id]
  );

  const filteredMemories = React.useMemo(() => {
    const q = memoryQuery.trim().toLowerCase();
    const list = q
      ? agentMemories.filter((m) =>
          [m.title, m.body, ...(m.tags ?? [])].some((f) => String(f).toLowerCase().includes(q))
        )
      : agentMemories;
    return [...list].sort((a, b) => {
      if (a.pinned !== b.pinned) return Number(b.pinned) - Number(a.pinned);
      return String(b.lastUsedAt).localeCompare(String(a.lastUsedAt));
    });
  }, [agentMemories, memoryQuery]);

  if (!agent) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <EmptyState
          icon={Search}
          title="Agent not found"
          description="It may have been deleted."
          action={
            <Button variant="secondary" size="sm" onClick={onBack}>
              Back to agents
            </Button>
          }
        />
      </div>
    );
  }

  const statusMeta = AGENT_STATUS_META[agent.status] ?? AGENT_STATUS_META.offline;
  const agentRoutines = routines.filter((r) => r.agentId === agent.id);
  const agentActivity = activity.filter((a) => a.agentId === agent.id);
  const promptDirty = prompt !== (agent.systemPrompt ?? "");

  function message() {
    const existing = threads
      .filter((t) => t.agentId === agent.id)
      .sort((a, b) => String(b.updatedAt).localeCompare(String(a.updatedAt)))[0];
    if (existing) openThread(existing.id);
    else createThread(agent.id);
    setView("chat");
  }

  function runFirstRoutine() {
    const target = agentRoutines.find((r) => r.enabled) ?? agentRoutines[0];
    if (!target) {
      toast({
        title: "No routines yet",
        description: `${agent.name} has nothing scheduled. Create one first.`,
        variant: "warning",
      });
      return;
    }
    runRoutine(target.id);
    toast({ title: `Running ${target.name}`, variant: "success" });
  }

  /**
   * Protection, on and off, from the one place that may do it.
   *
   * Every other door is closed: the tools refuse, the file guard refuses, and
   * the remove call refuses. This switch is the authorisation, and it is a
   * person clicking it - which is the whole distinction the flag encodes.
   */
  function toggleProtected() {
    const next = !isProtected(agent);
    saveAgent(agent.id, { protected: next });
    toast({
      title: next ? `${agent.name} is protected` : `${agent.name} is no longer protected`,
      description: next
        ? "Agents cannot edit or delete it. You still can, from here."
        : "It can be edited and deleted again, by you and by agents that may.",
      variant: next ? "success" : "warning",
    });
  }

  function togglePause() {
    const next = agent.status === "offline" ? "idle" : "offline";
    saveAgent(agent.id, { status: next });
    // What pausing actually does, said once, where the decision is made. It
    // used to be a badge and nothing else, so it is worth being plain about
    // both halves of it: no new work starts, and work already running is left
    // to finish rather than being killed underneath whoever started it.
    toast({
      title: next === "offline" ? `${agent.name} paused` : `${agent.name} resumed`,
      description:
        next === "offline"
          ? "New messages and scheduled routines will be refused. A turn already running finishes."
          : "Messages and scheduled routines will run again.",
    });
  }

  function duplicate() {
    createAgent({
      name: `${agent.name} copy`,
      role: agent.role,
      description: agent.description,
      systemPrompt: agent.systemPrompt,
      model: agent.model,
      computerId: agent.computerId,
      tags: agent.tags,
      avatarColor: agent.avatarColor,
      icon: agent.icon,
    });
    toast({ title: `Duplicated ${agent.name}`, variant: "success" });
  }

  const computerChoices = [
    { value: "__none", label: "No computer", description: "Chat only." },
    ...computers.map((c) => ({
      value: c.id,
      label: c.name,
      icon: <Icon name="Monitor" />,
      description: `${c.kind === "team" ? "Team" : "Private"} · ${c.os}`,
    })),
  ];

  return (
    <>
      {/* ── header band ─────────────────────────────────────────────────── */}
      <div className="flex h-14 shrink-0 items-center gap-2 px-4 sm:px-6">
        <IconButton size="lg" label="Back to agents" onClick={onBack}>
          <ArrowLeft />
        </IconButton>
        <span className="text-sm font-medium text-foreground">Agents</span>
        <span aria-hidden="true" className="text-sm text-muted-foreground">
          /
        </span>
        <span className="min-w-0 truncate text-sm text-muted-foreground">{agent.name}</span>
      </div>

      <ScrollArea className="flex-1">
        <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 px-4 pb-10 sm:px-6">
          <div className="flex flex-col gap-4 sm:flex-row sm:items-start">
            <AgentAvatar agent={agent} size="xl" showStatus />

            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <h1 className="text-[20px] leading-snug font-medium text-foreground">{agent.name}</h1>
                <span className="text-[13px] text-muted-foreground">{agent.handle}</span>
                <Badge variant={STATUS_VARIANT[agent.status] ?? "neutral"} dot>
                  {statusMeta.label}
                </Badge>
              </div>
              <p className="mt-1 flex items-center gap-1.5 text-[13px] text-muted-foreground">
                {isProtected(agent) ? (
                  <span
                    title="Agents cannot edit or delete this one."
                    className="inline-flex items-center gap-1 text-muted-foreground"
                  >
                    <ShieldCheck className="size-3" aria-hidden="true" />
                    Protected
                    <span aria-hidden="true">·</span>
                  </span>
                ) : null}
                {agent.role} · {agent.lastActiveAt ? `active ${relativeTime(agent.lastActiveAt)}` : "not used yet"}
              </p>
            </div>

            <div className="flex shrink-0 items-center gap-2">
              <Button variant="primary" size="sm" onClick={message}>
                <MessageSquare />
                Message
              </Button>
              <Button variant="secondary" size="sm" onClick={runFirstRoutine}>
                <Play />
                Run routine
              </Button>
              <DropdownMenu
                align="end"
                ariaLabel={`${agent.name} actions`}
                trigger={
                  <IconButton size="lg" label="More actions">
                    <MoreHorizontal />
                  </IconButton>
                }
              >
                <MenuItem icon={Settings2} onSelect={() => setEditorOpen(true)}>
                  Edit agent
                </MenuItem>
                <MenuItem icon={Copy} onSelect={duplicate}>
                  Duplicate
                </MenuItem>
                <MenuItem
                  icon={agent.status === "offline" ? Play : Pause}
                  onSelect={togglePause}
                >
                  {agent.status === "offline" ? "Resume" : "Pause"}
                </MenuItem>
                <MenuItem icon={isProtected(agent) ? ShieldAlert : ShieldCheck} onSelect={toggleProtected}>
                  {isProtected(agent) ? "Remove protection" : "Protect from agents"}
                </MenuItem>
                <MenuSeparator />
                {/* Deleting is not merely refused further down; the button that
                    starts it is gone, because a control that always fails is a
                    worse answer than no control. */}
                <MenuItem
                  icon={Trash2}
                  danger
                  disabled={isProtected(agent)}
                  onSelect={() => setConfirmDelete(true)}
                >
                  {isProtected(agent) ? "Delete (protected)" : "Delete"}
                </MenuItem>
              </DropdownMenu>
            </div>
          </div>

          {/* ── stats strip: whitespace and surface, no dividers ───────────── */}
          <div className="grid grid-cols-2 gap-4 rounded-2xl card-surface-subtle px-5 py-4 sm:grid-cols-4 sm:gap-8">
            <Stat label="Messages" value={formatCompactNumber(agent.stats?.messages ?? 0)} />
            <Stat label="Routines run" value={formatCompactNumber(agent.stats?.routinesRun ?? 0)} />
            <Stat label="Tokens used" value={formatCompactNumber(agent.stats?.tokensUsed ?? 0)} />
            <Stat label="Age" value={ageOf(agent)} />
          </div>

          <Tabs
            variant="strip"
            value={tab}
            onChange={setTab}
            ariaLabel="Agent detail sections"
            idPrefix={`agent-${agent.id}`}
            items={TABS.map((t) => ({
              ...t,
              icon: <Icon name={t.icon} />,
              badge:
                t.value === "routines"
                  ? agentRoutines.length || undefined
                  : t.value === "memory"
                    ? agentMemories.length || undefined
                    : undefined,
            }))}
          />

          {/* ── overview ───────────────────────────────────────────────────── */}
          <TabPanel value="overview" activeValue={tab} idPrefix={`agent-${agent.id}`} className="animate-fade-in">
            <div className="flex flex-col gap-6">
              <section>
                <h2 className="text-[11px] font-semibold text-muted-foreground">About</h2>
                <p className="mt-2 max-w-[var(--content-max)] text-[13px] leading-relaxed text-foreground/90">
                  {agent.description || "No description yet."}
                </p>
              </section>

              <section>
                <div className="flex items-center gap-2">
                  <h2 className="text-[11px] font-semibold text-muted-foreground">System prompt</h2>
                  <Button
                    variant={promptDirty ? "primary" : "subtle"}
                    size="xs"
                    className="ml-auto"
                    disabled={!promptDirty}
                    onClick={() => {
                      saveAgent(agent.id, { systemPrompt: prompt });
                      toast({ title: "System prompt saved", variant: "success" });
                    }}
                  >
                    Save
                  </Button>
                </div>
                <Textarea
                  autoResize
                  maxRows={20}
                  rows={8}
                  aria-label="System prompt"
                  className="mt-2 font-mono text-xs leading-relaxed"
                  value={prompt}
                  onChange={(e) => setPrompt(e.target.value)}
                />
              </section>

              <section className="grid gap-4 sm:grid-cols-2">
                <div>
                  <p className="text-[11px] font-semibold text-muted-foreground">Model</p>
                  <div className="mt-1.5">
                    <Select
                      size="xs"
                      ariaLabel="Model"
                      value={chatModels?.some((m) => m.ref === agent.model) ? agent.model : ""}
                      options={models}
                      emptyLabel="No models configured yet"
                      onChange={(v) => {
                        saveAgent(agent.id, { model: v });
                        toast({ title: "Model updated" });
                      }}
                    />
                  </div>
                </div>
                <div>
                  <p className="text-[11px] font-semibold text-muted-foreground">Computer</p>
                  <div className="mt-1.5">
                    <Select
                      size="xs"
                      ariaLabel="Computer"
                      value={agent.computerId ?? "__none"}
                      options={computerChoices}
                      onChange={(v) => {
                        saveAgent(agent.id, { computerId: v === "__none" ? null : v });
                        toast({ title: "Computer updated" });
                      }}
                    />
                  </div>
                </div>
              </section>

              <section>
                {/* This was a second list of switches called "Capabilities",
                    naming seven things - Browser, Desktop, Voice, Web search
                    among them - that no tool in the app has ever provided and
                    that nothing in the main process read. What an agent may
                    actually reach for is its permission rules, which have a tab
                    of their own a few pixels away. Two controls for one idea,
                    and the prettier one did nothing. */}
                <h2 className="text-[11px] font-semibold text-muted-foreground">Tools</h2>
                <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">
                  {allowedCount === totalTools
                    ? `Every tool the workspace allows, ${totalTools} of them.`
                    : `${allowedCount} of ${totalTools} tools, the rest turned off for this agent.`}
                </p>
                <Button
                  size="xs"
                  variant="subtle"
                  className="mt-2"
                  onClick={() => setTab("permissions")}
                >
                  <ShieldCheck />
                  Permissions
                </Button>
              </section>

              <section>
                <h2 className="text-[11px] font-semibold text-muted-foreground">Tags</h2>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {(agent.tags ?? []).length ? (
                    agent.tags.map((tag) => (
                      <span
                        key={tag}
                        className="inline-flex h-6 items-center rounded-full fill-secondary px-2.5 text-[11px] text-muted-foreground"
                      >
                        {tag}
                      </span>
                    ))
                  ) : (
                    <span className="text-[13px] text-muted-foreground">No tags.</span>
                  )}
                </div>
              </section>
            </div>
          </TabPanel>

          {/* ── routines ───────────────────────────────────────────────────── */}
          <TabPanel value="routines" activeValue={tab} idPrefix={`agent-${agent.id}`} className="animate-fade-in">
            {agentRoutines.length ? (
              <div className="flex flex-col gap-1">
                {agentRoutines.map((routine) => (
                  <RoutineRow
                    key={routine.id}
                    routine={routine}
                    compact
                    onOpen={() => setView("routines")}
                  />
                ))}
              </div>
            ) : (
              <EmptyState
                icon={Play}
                title="No routines"
                description={`${agent.name} has nothing on a schedule yet.`}
                action={
                  <Button variant="primary" size="sm" onClick={() => setView("routines")}>
                    <Plus />
                    Create routine
                  </Button>
                }
              />
            )}
          </TabPanel>

          {/* ── memory ─────────────────────────────────────────────────────── */}
          <TabPanel value="memory" activeValue={tab} idPrefix={`agent-${agent.id}`} className="animate-fade-in">
            <div className="flex flex-col gap-4">
              <div className="flex items-center gap-2">
                <SearchInput
                  size="xs"
                  className="max-w-64"
                  aria-label="Search memories"
                  placeholder="Search memories"
                  value={memoryQuery}
                  onChange={(e) => setMemoryQuery(e.target.value)}
                  onClear={() => setMemoryQuery("")}
                />
                <Button
                  variant="subtle"
                  size="xs"
                  className="ml-auto"
                  onClick={() => setAddingMemory(true)}
                >
                  <Plus />
                  Add memory
                </Button>
              </div>

              {filteredMemories.length ? (
                <div className="grid gap-3 sm:grid-cols-2">
                  {filteredMemories.map((memory) => (
                    <MemoryCard
                      key={memory.id}
                      memory={memory}
                      compact
                      className="animate-slide-up"
                      onPin={(m) => pinMemory(m.id)}
                      onOpen={() => setView("memory")}
                    />
                  ))}
                </div>
              ) : (
                <EmptyState
                  icon={Search}
                  title={memoryQuery ? "No matching memories" : "No memories yet"}
                  description={
                    memoryQuery
                      ? "Try a shorter query or a different tag."
                      : `${agent.name} has not learned anything durable yet.`
                  }
                  action={
                    memoryQuery ? undefined : (
                      <Button variant="primary" size="sm" onClick={() => setAddingMemory(true)}>
                        <Plus />
                        Add memory
                      </Button>
                    )
                  }
                />
              )}
            </div>
          </TabPanel>

          {/* ── permissions ────────────────────────────────────────────────── */}
          <TabPanel value="permissions" activeValue={tab} idPrefix={`agent-${agent.id}`} className="animate-fade-in">
            <PermissionMatrix agentId={agent.id} />
          </TabPanel>

          {/* ── activity ───────────────────────────────────────────────────── */}
          <TabPanel value="activity" activeValue={tab} idPrefix={`agent-${agent.id}`} className="animate-fade-in">
            {agentActivity.length ? (
              <div className="flex flex-col gap-1">
                {agentActivity.slice(0, ACTIVITY_CAP).map((event) => (
                  <ActivityRow key={event.id} event={event} />
                ))}
                {agentActivity.length > ACTIVITY_CAP ? (
                  <div className="mt-2">
                    <Button variant="ghost" size="sm" onClick={() => setView("activity")}>
                      View all {agentActivity.length} events
                    </Button>
                  </div>
                ) : null}
              </div>
            ) : (
              <EmptyState
                icon={Search}
                title="Nothing logged"
                description={`${agent.name} has not done anything recorded yet.`}
              />
            )}
          </TabPanel>
        </div>
      </ScrollArea>

      <AgentEditorDialog open={editorOpen} onOpenChange={setEditorOpen} agentId={agent.id} />

      <NewMemoryDialog
        open={addingMemory}
        onOpenChange={setAddingMemory}
        agentId={agent.id}
        onCreated={(id, title) =>
          toast({ variant: "success", title: `${agent.name} will remember this`, description: title })
        }
      />

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        destructive
        title={`Delete ${agent.name}?`}
        description="Its threads, memories and routines are kept, but the teammate is removed."
        confirmLabel="Delete agent"
        onConfirm={() => {
          deleteAgent(agent.id);
          setConfirmDelete(false);
          toast({ title: `${agent.name} deleted`, variant: "warning" });
          onBack?.();
        }}
      />
    </>
  );
}
