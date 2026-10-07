import * as React from "react";
import {
  Copy,
  LayoutGrid,
  List,
  MessageSquare,
  MoreHorizontal,
  Pause,
  Play,
  Plus,
  Search,
  Settings2,
  Trash2,
  Icon,
} from "@/components/icons";
import { AGENT_STATUS_META, getModel, relativeTime } from "@/data";
import { useApp } from "@/lib/store";
import { ANY, asRules, evaluate } from "@shared/permission";
import { parseModelRef } from "@shared/providers";
import { groupedTools } from "@shared/tools";
import { PREF, usePersistentState } from "@/lib/persist";
import { useToast } from "@/components/ui/toast";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { DropdownMenu, MenuItem, MenuSeparator } from "@/components/ui/dropdown-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { IconButton } from "@/components/ui/icon-button";
import { SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Segmented } from "@/components/ui/segmented";
import { Select } from "@/components/ui/select";
import { Tooltip } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { AgentDetail } from "@/features/agents/AgentDetail";
import { AgentEditorDialog } from "@/features/agents/AgentEditorDialog";
import { GUTTER, HEADER_GAP, ROW } from "@/components/layout/View";

const STATUS_VARIANT = {
  online: "success",
  busy: "warning",
  idle: "neutral",
  offline: "neutral",
};

const STATUS_OPTIONS = [
  { value: "all", label: "All statuses" },
  ...Object.entries(AGENT_STATUS_META).map(([value, meta]) => ({
    value,
    label: meta.label,
    icon: <Icon name={meta.icon} />,
  })),
];

/** The per-agent overflow menu - identical in grid and list so muscle memory holds. */
function AgentMenu({ agent, onOpenChat, onEdit, onDuplicate, onPause, onDelete, className }) {
  return (
    <DropdownMenu
      align="end"
      ariaLabel={`${agent.name} actions`}
      trigger={
        <IconButton
          size="lg"
          label={`${agent.name} actions`}
          className={className}
          onClick={(e) => e.stopPropagation()}
        >
          <MoreHorizontal />
        </IconButton>
      }
    >
      <MenuItem icon={MessageSquare} onSelect={onOpenChat}>
        Open chat
      </MenuItem>
      <MenuItem icon={Settings2} onSelect={onEdit}>
        Edit
      </MenuItem>
      <MenuItem icon={Copy} onSelect={onDuplicate}>
        Duplicate
      </MenuItem>
      <MenuItem icon={agent.status === "offline" ? Play : Pause} onSelect={onPause}>
        {agent.status === "offline" ? "Resume" : "Pause"}
      </MenuItem>
      <MenuSeparator />
      <MenuItem icon={Trash2} danger onSelect={onDelete}>
        Delete
      </MenuItem>
    </DropdownMenu>
  );
}

/**
 * The tools an agent may reach for, as icons.
 *
 * Read off the merged permission rules rather than a field on the agent. The
 * field it used to read named seven things, four of which no tool in the app
 * has ever provided, so a card could advertise a Browser the agent did not have.
 */
function ToolRow({ agentId }) {
  const { permissions } = useApp();
  const shown = React.useMemo(() => {
    const rules = [...asRules(permissions?.workspace), ...asRules(permissions?.agents?.[agentId])];
    return groupedTools()
      .flatMap((group) => group.tools)
      .filter((tool) => {
        const verdict = evaluate(rules, tool.key, ANY);
        return !(verdict.action === "deny" && (verdict.rule?.pattern ?? ANY) === ANY);
      })
      .slice(0, 5);
  }, [permissions, agentId]);

  if (!shown.length) return null;
  return (
    <div className="flex items-center gap-1">
      {shown.map((tool) => (
        <Tooltip key={tool.key} content={tool.label}>
          <span className="grid size-6 place-items-center rounded-full text-muted-foreground transition-colors duration-150 ease-out group-hover:text-foreground">
            <Icon name={tool.icon ?? "Wrench"} className="size-3.5" aria-hidden="true" />
          </span>
        </Tooltip>
      ))}
    </div>
  );
}

/**
 * What the card calls the model.
 *
 * A configured model arrives as `<providerId>/<modelId>`, and the provider
 * half is the part nobody is reading here - every agent in the list is likely
 * to name the same provider, and it is what pushed the pill past the edge of
 * the card. The card already sits inside a workspace where the provider is a
 * settings question; which model is the question the card answers.
 *
 * The demo catalogue's own name wins when there is one, because "Claude
 * Sonnet 4" beats the id it is stored under.
 */
function modelLabel(ref) {
  return getModel(ref)?.name ?? parseModelRef(ref).modelId ?? ref;
}

function AgentCard({ agent, routineCount, onOpen, menu }) {
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
      className={cn(
        "group relative flex animate-slide-up cursor-pointer flex-col gap-3 rounded-2xl card-surface-subtle p-4 text-left outline-none",
        "transition-colors duration-150 ease-out hover:card-surface-raised",
        "focus-visible:fill-control-hover"
      )}
    >
      <div className="flex items-start gap-3">
        <AgentAvatar agent={agent} size="lg" showStatus />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-foreground">{agent.name}</p>
          <p className="truncate text-[11px] text-muted-foreground">{agent.handle}</p>
        </div>
        <div className="absolute top-3 right-3 opacity-0 transition-opacity duration-150 ease-out group-hover:opacity-100 group-focus-within:opacity-100">
          {menu}
        </div>
      </div>

      <p className="text-[11px] font-medium text-muted-foreground">{agent.role}</p>

      <p className="line-clamp-2 text-[13px] leading-relaxed text-muted-foreground">
        {agent.description}
      </p>

      <div className="mt-auto flex items-center gap-2 pt-1">
        <ToolRow agentId={agent.id} />
        <span
          title={agent.model}
          className="ml-auto inline-flex h-5 min-w-0 items-center truncate rounded-full fill-secondary px-2 text-[10px] text-muted-foreground"
        >
          {modelLabel(agent.model)}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-x-3 text-[11px] text-muted-foreground">
        <span className="tabular-nums">{agent.stats?.messages ?? 0} messages</span>
        <span className="tabular-nums">
          {routineCount} {routineCount === 1 ? "routine" : "routines"}
        </span>
        <span className="ml-auto">{relativeTime(agent.lastActiveAt)}</span>
      </div>
    </div>
  );
}

function AgentListRow({ agent, onOpen, menu }) {
  const statusMeta = AGENT_STATUS_META[agent.status] ?? AGENT_STATUS_META.offline;
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
      className={cn(
        "group flex animate-slide-up cursor-pointer items-center gap-3 outline-none",
        ROW,
        "transition-colors duration-150 ease-out hover:fill-nav",
        "focus-visible:fill-nav"
      )}
    >
      <AgentAvatar agent={agent} size="md" showStatus />

      <div className="min-w-0 w-44 shrink-0">
        <p className="truncate text-[13px] font-medium text-foreground">{agent.name}</p>
        <p className="truncate text-[11px] text-muted-foreground">{agent.handle}</p>
      </div>

      <p className="hidden w-40 shrink-0 truncate text-[11px] text-muted-foreground md:block">
        {agent.role}
      </p>

      <p className="hidden min-w-0 flex-1 truncate text-[13px] text-muted-foreground lg:block">
        {agent.description}
      </p>

      <div className="ml-auto hidden shrink-0 items-center gap-1 xl:flex">
        <ToolRow agentId={agent.id} />
      </div>

      <span
        title={agent.model}
        className="hidden h-5 max-w-40 shrink-0 items-center truncate rounded-full fill-secondary px-2 text-[10px] text-muted-foreground sm:inline-flex"
      >
        {modelLabel(agent.model)}
      </span>

      <Badge
        variant={STATUS_VARIANT[agent.status] ?? "neutral"}
        size="sm"
        dot
        className="hidden shrink-0 sm:inline-flex"
      >
        {statusMeta.label}
      </Badge>

      <span className="hidden w-20 shrink-0 text-right text-[11px] tabular-nums text-muted-foreground sm:block">
        {relativeTime(agent.lastActiveAt)}
      </span>

      <div className="shrink-0 opacity-0 transition-opacity duration-150 ease-out group-hover:opacity-100 group-focus-within:opacity-100">
        {menu}
      </div>
    </div>
  );
}

export function AgentsView() {
  const {
    agents,
    threads,
    routines,
    activeAgentId,
    setActiveAgentId,
    createThread,
    openThread,
    createAgent,
    saveAgent,
    deleteAgent,
    setView,
  } = useApp();
  const { toast } = useToast();

  // No URL router exists - the detail pane is local routing over `activeAgentId`.
  const [detailOpen, setDetailOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [layout, setLayout] = usePersistentState(
    PREF.agentsLayout,
    "grid",
    (v) => v === "grid" || v === "list"
  );
  const [status, setStatus] = usePersistentState(PREF.agentsStatus, "all", (v) =>
    STATUS_OPTIONS.some((o) => o.value === v)
  );
  const [editorAgentId, setEditorAgentId] = React.useState(undefined);
  const [editorOpen, setEditorOpen] = React.useState(false);
  const [pendingDelete, setPendingDelete] = React.useState(null);

  const visible = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    return agents.filter((agent) => {
      if (status !== "all" && agent.status !== status) return false;
      if (!q) return true;
      return [agent.name, agent.handle, agent.role, agent.description, ...(agent.tags ?? [])].some(
        (f) =>
          String(f ?? "")
            .toLowerCase()
            .includes(q)
      );
    });
  }, [agents, query, status]);

  if (detailOpen && activeAgentId) {
    return <AgentDetail agentId={activeAgentId} onBack={() => setDetailOpen(false)} />;
  }

  function openDetail(agentId) {
    setActiveAgentId(agentId);
    setDetailOpen(true);
  }

  function openChat(agent) {
    const existing = threads
      .filter((t) => t.agentId === agent.id)
      .sort((a, b) => String(b.updatedAt).localeCompare(String(a.updatedAt)))[0];
    if (existing) openThread(existing.id);
    else createThread(agent.id);
    setView("chat");
  }

  function duplicate(agent) {
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

  function togglePause(agent) {
    const next = agent.status === "offline" ? "idle" : "offline";
    saveAgent(agent.id, { status: next });
    toast({
      title: next === "offline" ? `${agent.name} paused` : `${agent.name} resumed`,
    });
  }

  const menuFor = (agent, className) => (
    <AgentMenu
      agent={agent}
      className={className}
      onOpenChat={() => openChat(agent)}
      onEdit={() => {
        setEditorAgentId(agent.id);
        setEditorOpen(true);
      }}
      onDuplicate={() => duplicate(agent)}
      onPause={() => togglePause(agent)}
      onDelete={() => setPendingDelete(agent)}
    />
  );

  const onlineCount = agents.filter((b) => b.status === "online" || b.status === "busy").length;

  return (
    <>
      {/* ── header ──────────────────────────────────────────────────────── */}
      <div className={cn("flex h-14 shrink-0 items-center", HEADER_GAP, GUTTER)}>
        <h1 className="shrink-0 text-sm font-medium text-foreground">Agents</h1>
        <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
          {visible.length === agents.length
            ? `${agents.length} · ${onlineCount} active`
            : `${visible.length} of ${agents.length}`}
        </span>

        <div className="ml-auto flex items-center gap-2">
          <SearchInput
            size="sm"
            className="w-40 sm:w-56"
            aria-label="Search agents"
            placeholder="Search agents"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onClear={() => setQuery("")}
          />
          <Select
            size="sm"
            className="w-36"
            ariaLabel="Filter by status"
            value={status}
            options={STATUS_OPTIONS}
            onChange={setStatus}
          />
          <Segmented
            size="xs"
            label="View layout"
            value={layout}
            onChange={setLayout}
            options={[
              { value: "grid", icon: <LayoutGrid />, label: null },
              { value: "list", icon: <List />, label: null },
            ]}
          />
          <Button
            variant="primary"
            size="xs"
            onClick={() => {
              setEditorAgentId(undefined);
              setEditorOpen(true);
            }}
          >
            <Plus />
            New agent
          </Button>
        </div>
      </div>

      {/* ── body ────────────────────────────────────────────────────────── */}
      <ScrollArea className="flex-1">
        <div className={cn("pb-10", GUTTER)}>
          {visible.length === 0 ? (
            <EmptyState
              className="py-20"
              icon={Search}
              title="No agents match"
              description={
                query
                  ? `Nothing matches “${query}”${status !== "all" ? " with that status" : ""}.`
                  : "No agents have that status right now."
              }
              action={
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => {
                    setQuery("");
                    setStatus("all");
                  }}
                >
                  Clear filters
                </Button>
              }
            />
          ) : layout === "grid" ? (
            <div
              key="grid"
              className="grid animate-fade-in gap-3 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4"
            >
              {visible.map((agent) => (
                <AgentCard
                  key={agent.id}
                  agent={agent}
                  routineCount={routines.filter((r) => r.agentId === agent.id).length}
                  onOpen={() => openDetail(agent.id)}
                  menu={menuFor(agent)}
                />
              ))}
            </div>
          ) : (
            <div key="list" className="flex animate-fade-in flex-col">
              {visible.map((agent) => (
                <AgentListRow
                  key={agent.id}
                  agent={agent}
                  routineCount={routines.filter((r) => r.agentId === agent.id).length}
                  onOpen={() => openDetail(agent.id)}
                  menu={menuFor(agent)}
                />
              ))}
            </div>
          )}
        </div>
      </ScrollArea>

      <AgentEditorDialog open={editorOpen} onOpenChange={setEditorOpen} agentId={editorAgentId} />

      <ConfirmDialog
        open={Boolean(pendingDelete)}
        onOpenChange={(next) => !next && setPendingDelete(null)}
        destructive
        title={pendingDelete ? `Delete ${pendingDelete.name}?` : "Delete agent?"}
        description="The teammate is removed from Inertia. Its threads stay searchable."
        confirmLabel="Delete agent"
        onConfirm={() => {
          const agent = pendingDelete;
          if (agent) {
            deleteAgent(agent.id);
            toast({ title: `${agent.name} deleted`, variant: "warning" });
          }
          setPendingDelete(null);
        }}
      />
    </>
  );
}
