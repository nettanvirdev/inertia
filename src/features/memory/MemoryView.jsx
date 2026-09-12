import * as React from "react";
import { Pin, Plus, Search, Trash2, X, Icon } from "@/components/icons";
import { MEMORY_KIND_META, relativeTime } from "@/data";
import { useApp } from "@/lib/store";
import { PREF, usePersistentState } from "@/lib/persist";
import { useToast } from "@/components/ui/toast";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Dialog, DialogBody, DialogFooter, DialogTitle } from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input, SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tabs } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { MemoryCard } from "@/features/memory/MemoryCard";
import { NewMemoryDialog } from "@/features/memory/NewMemoryDialog";
import { GUTTER, HEADER_GAP } from "@/components/layout/View";

const SORTS = [
  { value: "recent", label: "Recently used" },
  { value: "used", label: "Most used" },
  { value: "confidence", label: "Confidence" },
];

const KIND_TABS = [
  { value: "all", label: "All" },
  ...Object.entries(MEMORY_KIND_META).map(([value, meta]) => ({
    value,
    label: meta.label,
    icon: <Icon name={meta.icon} />,
  })),
];

function sortMemories(list, sort) {
  const next = [...list];
  if (sort === "used") next.sort((a, b) => b.useCount - a.useCount);
  else if (sort === "confidence") next.sort((a, b) => b.confidence - a.confidence);
  else next.sort((a, b) => String(b.lastUsedAt).localeCompare(String(a.lastUsedAt)));
  return next;
}

/** The edit form lives in its own dialog so the grid never re-renders while typing. */
function MemoryDialog({ memory, open, onOpenChange, agentName, onSave, onDelete }) {
  const [draft, setDraft] = React.useState(null);
  const [tagDraft, setTagDraft] = React.useState("");

  React.useEffect(() => {
    if (!open || !memory) return;
    setTagDraft("");
    setDraft({
      kind: memory.kind,
      title: memory.title,
      body: memory.body,
      tags: memory.tags ?? [],
      pinned: Boolean(memory.pinned),
    });
  }, [open, memory?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!memory || !draft) return null;

  const patch = (next) => setDraft((d) => ({ ...d, ...next }));

  const addTag = (raw) => {
    const tag = raw.trim().replace(/^#/, "").toLowerCase();
    if (!tag) return;
    setDraft((d) => (d.tags.includes(tag) ? d : { ...d, tags: [...d.tags, tag] }));
    setTagDraft("");
  };
  const removeTag = (tag) => setDraft((d) => ({ ...d, tags: d.tags.filter((t) => t !== tag) }));

  return (
    <Dialog open={open} onOpenChange={onOpenChange} size="lg">
      <DialogTitle>Memory</DialogTitle>

      <DialogBody>
        <ScrollArea className="max-h-[min(30rem,calc(100dvh-16rem))] -mx-1 px-1">
          <div className="flex flex-col gap-4 pb-1">
            <p className="text-[11px] text-muted-foreground">
              {agentName} · {memory.useCount} uses · last recalled {relativeTime(memory.lastUsedAt)} ·{" "}
              {Math.round(memory.confidence * 100)}% confidence
            </p>

            <div className="grid gap-3 sm:grid-cols-[10rem_1fr]">
              <div className="flex flex-col">
                <label className="text-xs text-foreground/90">Kind</label>
                <div className="mt-1">
                  <Select
                    size="xs"
                    ariaLabel="Memory kind"
                    value={draft.kind}
                    options={Object.entries(MEMORY_KIND_META).map(([value, meta]) => ({
                      value,
                      label: meta.label,
                      icon: <Icon name={meta.icon} />,
                    }))}
                    onChange={(v) => patch({ kind: v })}
                  />
                </div>
              </div>
              <div className="flex flex-col">
                <label htmlFor="mem-title" className="text-xs text-foreground/90">
                  Title
                </label>
                <div className="mt-1">
                  <Input
                    id="mem-title"
                    size="xs"
                    value={draft.title}
                    onChange={(e) => patch({ title: e.target.value })}
                  />
                </div>
              </div>
            </div>

            <div className="flex flex-col">
              <label htmlFor="mem-body" className="text-xs text-foreground/90">
                Body
              </label>
              <div className="mt-1">
                <Textarea
                  id="mem-body"
                  autoResize
                  maxRows={16}
                  rows={6}
                  className="text-[13px] leading-relaxed"
                  value={draft.body}
                  onChange={(e) => patch({ body: e.target.value })}
                />
              </div>
            </div>

            <div className="flex flex-col">
              <label htmlFor="mem-tags" className="text-xs text-foreground/90">
                Tags
              </label>
              <div className="mt-1 flex flex-wrap items-center gap-1.5">
                {draft.tags.map((tag) => (
                  <span
                    key={tag}
                    className="inline-flex h-6 items-center gap-1 rounded-full fill-secondary pr-1 pl-2.5 text-[11px] text-muted-foreground"
                  >
                    {tag}
                    <button
                      type="button"
                      aria-label={`Remove ${tag}`}
                      onClick={() => removeTag(tag)}
                      className={cn(
                        "grid size-4 place-items-center rounded-full outline-none",
                        "transition-colors duration-150 ease-out hover:fill-close hover:text-foreground",
                        "focus-visible:fill-close"
                      )}
                    >
                      <X className="size-3" aria-hidden="true" />
                    </button>
                  </span>
                ))}
                <Input
                  id="mem-tags"
                  size="xs"
                  className="w-40"
                  placeholder="Add a tag"
                  value={tagDraft}
                  onChange={(e) => setTagDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === ",") {
                      e.preventDefault();
                      addTag(tagDraft);
                    } else if (e.key === "Backspace" && !tagDraft && draft.tags.length) {
                      e.preventDefault();
                      removeTag(draft.tags[draft.tags.length - 1]);
                    }
                  }}
                  onBlur={() => addTag(tagDraft)}
                />
              </div>
            </div>

            <div className="flex h-7 items-center gap-2">
              <span className="text-xs text-foreground/90">Pinned</span>
              <p className="min-w-0 flex-1 text-[11px] text-muted-foreground">
                Pinned memories are always loaded into context.
              </p>
              <Switch
                checked={draft.pinned}
                onCheckedChange={(v) => patch({ pinned: v })}
                aria-label="Pinned"
              />
            </div>
          </div>
        </ScrollArea>
      </DialogBody>

      <DialogFooter className="sm:justify-between">
        <Button variant="danger" size="pill" onClick={onDelete}>
          <Trash2 />
          Delete
        </Button>
        <div className="flex flex-col-reverse gap-2 sm:flex-row">
          <Button variant="secondary" size="pill" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="primary" size="pill" onClick={() => onSave(draft)}>
            Save memory
          </Button>
        </div>
      </DialogFooter>
    </Dialog>
  );
}

export function MemoryView() {
  const { memories, agents, pinMemory, saveMemory, deleteMemory } = useApp();
  const { toast } = useToast();

  const [query, setQuery] = React.useState("");
  // a remembered agent that has since been deleted would pin this screen to zero results
  const [agentId, setAgentId] = usePersistentState(
    PREF.memoryAgent,
    "all",
    (v) => v === "all" || agents.some((b) => b.id === v)
  );
  const [kind, setKind] = usePersistentState(PREF.memoryKind, "all", (v) =>
    KIND_TABS.some((t) => t.value === v)
  );
  const [sort, setSort] = usePersistentState(PREF.memorySort, "recent", (v) =>
    SORTS.some((s) => s.value === v)
  );
  const [openId, setOpenId] = React.useState(null);
  const [confirmDeleteId, setConfirmDeleteId] = React.useState(null);
  const [creating, setCreating] = React.useState(false);

  const agentOptions = React.useMemo(
    () => [
      { value: "all", label: "All agents" },
      ...agents.map((b) => ({ value: b.id, label: b.name, description: b.role })),
    ],
    [agents]
  );

  const agentName = (id) => agents.find((b) => b.id === id)?.name ?? "Unassigned";

  const visible = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    return memories.filter((m) => {
      if (agentId !== "all" && m.agentId !== agentId) return false;
      if (kind !== "all" && m.kind !== kind) return false;
      if (!q) return true;
      return [m.title, m.body, ...(m.tags ?? [])].some((f) =>
        String(f ?? "").toLowerCase().includes(q)
      );
    });
  }, [memories, query, agentId, kind]);

  const pinned = sortMemories(visible.filter((m) => m.pinned), sort);
  const rest = sortMemories(visible.filter((m) => !m.pinned), sort);

  const openMemory = openId ? memories.find((m) => m.id === openId) : null;
  const pendingDelete = confirmDeleteId ? memories.find((m) => m.id === confirmDeleteId) : null;

  const cardProps = (memory) => ({
    memory,
    compact: true,
    // Cards are keyed by id, so only one that has just been added - or has
    // just come into the filter - is new enough to arrive; the rest hold.
    className: "animate-slide-up",
    onPin: (m) => pinMemory(m.id),
    onEdit: (m) => setOpenId(m.id),
    onDelete: (m) => setConfirmDeleteId(m.id),
    onOpen: (m) => setOpenId(m.id),
    // Approving is just clearing the flag. The record was written when it was
    // captured; what changes is whether anything is allowed to believe it.
    onApprove: (m) => saveMemory(m.id, { pending: false }),
  });

  const filtersActive = Boolean(query) || agentId !== "all" || kind !== "all";

  return (
    <>
      {/* ── header ──────────────────────────────────────────────────────── */}
      <div className={cn("flex h-14 shrink-0 items-center", HEADER_GAP, GUTTER)}>
        <h1 className="shrink-0 text-sm font-medium text-foreground">Memory</h1>
        <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
          {visible.length === memories.length
            ? `${memories.length} entries`
            : `${visible.length} of ${memories.length}`}
        </span>

        <div className="ml-auto flex items-center gap-2">
          <SearchInput
            size="sm"
            className="w-40 sm:w-56"
            aria-label="Search memories"
            placeholder="Search title, body, tags"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onClear={() => setQuery("")}
          />
          <Select
            size="sm"
            className="w-36"
            ariaLabel="Filter by agent"
            value={agentId}
            options={agentOptions}
            onChange={setAgentId}
          />
          <Select
            size="sm"
            className="w-36"
            ariaLabel="Sort memories"
            value={sort}
            options={SORTS}
            onChange={setSort}
          />
          <Button variant="primary" size="sm" onClick={() => setCreating(true)}>
            <Plus />
            Add memory
          </Button>
        </div>
      </div>

      <div className={cn("shrink-0 pb-3", GUTTER)}>
        <Tabs
          variant="pill"
          value={kind}
          onChange={setKind}
          ariaLabel="Filter by kind"
          idPrefix="memory-kind"
          items={KIND_TABS}
        />
      </div>

      {/* ── body ────────────────────────────────────────────────────────── */}
      <ScrollArea className="flex-1">
        <div className={cn("pb-10", GUTTER)}>
          {visible.length === 0 ? (
            <EmptyState
              className="py-20"
              icon={Search}
              title="No memories match"
              description={
                filtersActive
                  ? "Nothing here matches the current filters."
                  : "Agents write memories as they work. There is nothing stored yet."
              }
              action={
                filtersActive ? (
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => {
                      setQuery("");
                      setAgentId("all");
                      setKind("all");
                    }}
                  >
                    Clear filters
                  </Button>
                ) : (
                  <Button variant="primary" size="sm" onClick={() => setCreating(true)}>
                    <Plus />
                    Add memory
                  </Button>
                )
              }
            />
          ) : (
            <div className="flex flex-col gap-6">
              {pinned.length ? (
                <section>
                  <h2 className="flex items-center gap-1.5 text-[11px] font-semibold text-muted-foreground">
                    <Pin className="size-3 fill-current" aria-hidden="true" />
                    Pinned
                  </h2>
                  <div className="mt-2 grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
                    {pinned.map((memory) => (
                      <MemoryCard key={memory.id} {...cardProps(memory)} />
                    ))}
                  </div>
                </section>
              ) : null}

              {rest.length ? (
                <section>
                  {pinned.length ? (
                    <h2 className="text-[11px] font-semibold text-muted-foreground">Everything else</h2>
                  ) : null}
                  <div className={cn("grid gap-3 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4", pinned.length && "mt-2")}>
                    {rest.map((memory) => (
                      <MemoryCard key={memory.id} {...cardProps(memory)} />
                    ))}
                  </div>
                </section>
              ) : null}
            </div>
          )}
        </div>
      </ScrollArea>

      <NewMemoryDialog
        open={creating}
        onOpenChange={setCreating}
        agentId={agentId === "all" ? undefined : agentId}
        onCreated={(id, title) => toast({ variant: "success", title: "Memory added", description: title })}
      />

      <MemoryDialog
        memory={openMemory}
        open={Boolean(openMemory)}
        onOpenChange={(next) => !next && setOpenId(null)}
        agentName={openMemory ? agentName(openMemory.agentId) : ""}
        onSave={(draft) => {
          saveMemory(openMemory.id, draft);
          toast({ title: "Memory saved", variant: "success" });
          setOpenId(null);
        }}
        onDelete={() => {
          setConfirmDeleteId(openMemory.id);
          setOpenId(null);
        }}
      />

      <ConfirmDialog
        open={Boolean(pendingDelete)}
        onOpenChange={(next) => !next && setConfirmDeleteId(null)}
        destructive
        title="Delete this memory?"
        description={
          pendingDelete
            ? `“${pendingDelete.title}” is removed from ${agentName(pendingDelete.agentId)}'s long-term memory. This cannot be undone.`
            : ""
        }
        confirmLabel="Delete memory"
        onConfirm={() => {
          if (pendingDelete) {
            deleteMemory(pendingDelete.id);
            toast({ title: "Memory deleted", variant: "warning" });
          }
          setConfirmDeleteId(null);
        }}
      />
    </>
  );
}
