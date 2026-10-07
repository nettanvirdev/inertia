import * as React from "react";
import {
  FileSearch,
  LayoutGrid,
  Library as LibraryIcon,
  MessageCircle,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  Rows3,
  SearchX,
  Trash2,
  Icon,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { useNow } from "@/hooks/use-now";
import { PREF, usePersistentState } from "@/lib/persist";
import { useDateFormat, utcDayKey } from "@/lib/datetime";
import { LIBRARY_CATEGORIES, LIBRARY_CATEGORY_META, formatBytes, relativeTime } from "@/data";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { ContextMenu } from "@/components/ui/context-menu";
import {
  Dialog,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogFooter,
} from "@/components/ui/dialog";
import { DropdownMenu, MenuItem, MenuSeparator } from "@/components/ui/dropdown-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { IconButton } from "@/components/ui/icon-button";
import { Input, SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Segmented } from "@/components/ui/segmented";
import { Select } from "@/components/ui/select";
import { Tabs } from "@/components/ui/tabs";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { ArtifactDialog } from "@/features/library/ArtifactDialog";
import { ArtifactPreview } from "@/features/library/ArtifactPreview";
import { collectLibraryArtifacts } from "@/features/library/artifacts";
import { GUTTER, HEADER_GAP } from "@/components/layout/View";

/* ── small shared bits ─────────────────────────────────────────────────── */

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

const SORT_OPTIONS = [
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "largest", label: "Largest first" },
  { value: "name", label: "Name A-Z" },
];

const VIEW_OPTIONS = [
  { value: "grid", label: "Grid", icon: <LayoutGrid /> },
  { value: "list", label: "List", icon: <Rows3 /> },
];

const SOURCE_LABEL = {
  attachment: "Attached in chat",
  tool: "Produced by a tool",
  routine: "Written by a routine",
  upload: "Uploaded",
};

const SECTION_HEADING =
  "sticky top-0 z-10 bg-background px-2.5 pt-4 pb-1 text-[11px] font-semibold text-muted-foreground";

/** Sort keys, coerced. A record read off the folder can be missing any field,
 *  and `undefined.localeCompare` is a blank window. */
const text = (value) => (value == null ? "" : String(value));

/** The bucket a record with no readable timestamp lands in. `null` rather than
 *  a string, so it can never collide with a real day key. */
const UNDATED = null;

function dayLabel(dayKey, nowIso) {
  if (dayKey === UNDATED) return "Undated";
  const now = new Date(nowIso);
  const today = now.toISOString().slice(0, 10);
  const yesterday = new Date(now.getTime() - 86_400_000).toISOString().slice(0, 10);
  if (dayKey === today) return "Today";
  if (dayKey === yesterday) return "Yesterday";
  const d = new Date(`${dayKey}T00:00:00Z`);
  if (Number.isNaN(d.getTime())) return String(dayKey);
  const label = `${MONTHS[d.getUTCMonth()]} ${d.getUTCDate()}`;
  return d.getUTCFullYear() === now.getUTCFullYear() ? label : `${label}, ${d.getUTCFullYear()}`;
}

/* ── artifact card & row ───────────────────────────────────────────────── */

function MetaLine({ agent, thread }) {
  return (
    <span className="truncate text-[11px] text-muted-foreground">
      {agent?.name ?? "Workspace"}
      {thread ? ` · ${thread.title}` : ""}
    </span>
  );
}

function ArtifactCard({ item, agent, thread, onOpen }) {
  const meta = LIBRARY_CATEGORY_META[item.category];
  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={`${item.name}, ${meta.label}`}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
      className={cn(
        "flex cursor-pointer flex-col gap-2.5 rounded-2xl p-2.5 text-left outline-none",
        "transition-colors duration-150 ease-out hover:fill-nav",
        "focus-visible:fill-nav"
      )}
    >
      <ArtifactPreview artifact={item} className="h-40 shrink-0" compact />
      <div className="flex min-w-0 flex-col gap-1 px-0.5 pb-0.5">
        <span className="truncate text-[13px] text-foreground" title={item.name}>
          {item.name}
        </span>
        <MetaLine item={item} agent={agent} thread={thread} />
        <div className="mt-0.5 flex flex-wrap items-center gap-1.5">
          <Badge variant={meta.tone} size="sm">
            <Icon name={meta.icon} className="size-3" />
            {meta.label.replace(/s$/, "")}
          </Badge>
          <span className="text-[11px] tabular-nums text-muted-foreground">
            {formatBytes(item.sizeBytes)}
          </span>
          <span className="text-[11px] text-muted-foreground">·</span>
          <span className="text-[11px] text-muted-foreground">{relativeTime(item.createdAt)}</span>
        </div>
      </div>
    </div>
  );
}

function ArtifactRow({ item, agent, thread, onOpen }) {
  const meta = LIBRARY_CATEGORY_META[item.category];
  return (
    <button
      type="button"
      onClick={onOpen}
      className={cn(
        "flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left outline-none",
        "transition-colors duration-150 ease-out hover:fill-nav",
        "focus-visible:fill-nav"
      )}
    >
      <span className="grid size-8 shrink-0 place-items-center rounded-full fill-control text-muted-foreground">
        <Icon name={meta.icon} className="size-3.5" />
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-[13px] text-foreground" title={item.name}>
          {item.name}
        </span>
        <MetaLine item={item} agent={agent} thread={thread} />
      </span>
      <Badge variant={meta.tone} size="sm" className="hidden shrink-0 sm:inline-flex">
        {meta.label.replace(/s$/, "")}
      </Badge>
      <span className="hidden w-20 shrink-0 text-right text-[11px] tabular-nums text-muted-foreground md:block">
        {formatBytes(item.sizeBytes)}
      </span>
      <span className="w-20 shrink-0 text-right text-[11px] text-muted-foreground">
        {relativeTime(item.createdAt)}
      </span>
    </button>
  );
}

/* ── conversation row ──────────────────────────────────────────────────── */

function ThreadRow({ thread, agent, artifactCount, onOpen, onPin, onRename, onDelete }) {
  const menuItems = [
    {
      icon: thread.pinned ? PinOff : Pin,
      label: thread.pinned ? "Unpin" : "Pin",
      onSelect: onPin,
    },
    { icon: Pencil, label: "Rename", onSelect: onRename },
    { type: "separator" },
    { icon: Trash2, label: "Delete", danger: true, onSelect: onDelete },
  ];

  return (
    <ContextMenu items={menuItems}>
      <div
        className={cn(
          "group flex items-center gap-3 rounded-xl px-2.5 py-2",
          "transition-colors duration-150 ease-out hover:fill-nav"
        )}
      >
        <button
          type="button"
          onClick={onOpen}
          className="flex min-w-0 flex-1 items-center gap-3 rounded-lg text-left outline-none focus-visible:fill-nav"
        >
          <AgentAvatar agent={agent} size="sm" className="shrink-0" />
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate text-[13px] text-foreground" title={thread.title}>
              {thread.title}
            </span>
            <span className="truncate text-[11px] text-muted-foreground">
              {thread.preview || "No messages yet"}
            </span>
          </span>
        </button>

        <span className="hidden shrink-0 items-center gap-3 text-[11px] tabular-nums text-muted-foreground sm:flex">
          <span className="flex items-center gap-1">
            <MessageCircle className="size-3" aria-hidden="true" />
            {thread.messageCount ?? 0}
          </span>
          <span className="flex items-center gap-1">
            <LibraryIcon className="size-3" aria-hidden="true" />
            {artifactCount}
          </span>
        </span>
        <span className="w-16 shrink-0 text-right text-[11px] text-muted-foreground">
          {relativeTime(thread.updatedAt)}
        </span>

        <DropdownMenu
          side="bottom"
          align="end"
          ariaLabel={`Actions for ${thread.title}`}
          trigger={
            <IconButton
              size="sm"
              label={`Actions for ${thread.title}`}
              className="shrink-0 opacity-0 transition-opacity duration-150 ease-out group-hover:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100"
            >
              <MoreHorizontal />
            </IconButton>
          }
        >
          <MenuItem icon={thread.pinned ? PinOff : Pin} onSelect={onPin}>
            {thread.pinned ? "Unpin" : "Pin"}
          </MenuItem>
          <MenuItem icon={Pencil} onSelect={onRename}>
            Rename
          </MenuItem>
          <MenuSeparator />
          <MenuItem icon={Trash2} danger onSelect={onDelete}>
            Delete
          </MenuItem>
        </DropdownMenu>
      </div>
    </ContextMenu>
  );
}

/* ── the screen ────────────────────────────────────────────────────────── */

export function LibraryView() {
  const { threads, agents, messages, openThread, togglePinThread, renameThread, deleteThread } =
    useApp();
  const now = useNow();

  // The catalogue is the transcripts, read. It used to be a `useState` over a
  // hand-written roster, which meant a fresh workspace opened onto files nobody
  // had made and a deletion here vanished on unmount. Deriving it means a
  // message sent in chat shows up on this screen the moment it is sent, and
  // nothing on this screen can claim an artifact the conversation does not have.
  const items = React.useMemo(
    () => collectLibraryArtifacts({ threads, messages }),
    [threads, messages]
  );
  const [tab, setTab] = usePersistentState(
    PREF.libraryTab,
    "all",
    (v) => v === "all" || v === "conversation" || LIBRARY_CATEGORIES.includes(v)
  );
  const [query, setQuery] = React.useState("");
  // a remembered agent that has since been deleted would pin this screen to zero results
  const [agentId, setAgentId] = usePersistentState(
    PREF.libraryAgent,
    "all",
    (v) => v === "all" || agents.some((b) => b.id === v)
  );
  const [sort, setSort] = usePersistentState(PREF.librarySort, "newest", (v) =>
    SORT_OPTIONS.some((o) => o.value === v)
  );
  const [layout, setLayout] = usePersistentState(PREF.libraryLayout, "grid", (v) =>
    VIEW_OPTIONS.some((o) => o.value === v)
  );

  // The detail sheet is the one place in here that states a real date and
  // clock rather than "3d ago", so it is the one that has to honour the zone.
  const { formatDateTime } = useDateFormat();

  const [openItem, setOpenItem] = React.useState(null);
  const [deletingThread, setDeletingThread] = React.useState(null);
  const [renaming, setRenaming] = React.useState(null);
  const [renameValue, setRenameValue] = React.useState("");

  const agentById = React.useMemo(() => new Map(agents.map((b) => [b.id, b])), [agents]);
  const threadById = React.useMemo(() => new Map(threads.map((t) => [t.id, t])), [threads]);

  const artifactsByThread = React.useMemo(() => {
    const map = new Map();
    for (const item of items) map.set(item.threadId, (map.get(item.threadId) ?? 0) + 1);
    return map;
  }, [items]);

  const agentOptions = React.useMemo(
    () => [
      { value: "all", label: "All agents" },
      ...agents.map((b) => ({ value: b.id, label: b.name })),
    ],
    [agents]
  );

  /* filtering - query and agent apply everywhere, the tab only picks the category */
  const scoped = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    return items.filter((item) => {
      if (agentId !== "all" && item.agentId !== agentId) return false;
      if (!q) return true;
      const threadTitle = threadById.get(item.threadId)?.title ?? "";
      return (
        item.name.toLowerCase().includes(q) ||
        (item.summary ?? "").toLowerCase().includes(q) ||
        threadTitle.toLowerCase().includes(q)
      );
    });
  }, [items, query, agentId, threadById]);

  const scopedThreads = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    return threads.filter((t) => {
      if (t.temporary) return false;
      if (agentId !== "all" && t.agentId !== agentId) return false;
      if (!q) return true;
      return t.title.toLowerCase().includes(q) || (t.preview ?? "").toLowerCase().includes(q);
    });
  }, [threads, query, agentId]);

  const counts = React.useMemo(() => {
    const base = { all: scoped.length, conversation: scopedThreads.length };
    for (const c of LIBRARY_CATEGORIES) base[c] = 0;
    for (const item of scoped) if (base[item.category] != null) base[item.category] += 1;
    return base;
  }, [scoped, scopedThreads]);

  const tabItems = React.useMemo(
    () => [
      { value: "all", label: "All", icon: LibraryIcon, badge: counts.all },
      {
        value: "conversation",
        label: LIBRARY_CATEGORY_META.conversation.label,
        icon: MessageCircle,
        badge: counts.conversation,
      },
      ...LIBRARY_CATEGORIES.map((c) => ({
        value: c,
        label: LIBRARY_CATEGORY_META[c].label,
        badge: counts[c],
      })),
    ],
    [counts]
  );

  const visible = React.useMemo(() => {
    const list =
      tab === "all" || tab === "conversation" ? scoped : scoped.filter((i) => i.category === tab);
    const sorted = list.slice();
    // Every comparator reads a field a record can be missing, and a sort that
    // throws takes the screen with it. Coercing is the right answer here rather
    // than filtering: a nameless or undated artifact should sort to one end of
    // the list, not vanish from it.
    sorted.sort((a, b) => {
      if (sort === "oldest") return text(a.createdAt).localeCompare(text(b.createdAt));
      if (sort === "largest") return (Number(b.sizeBytes) || 0) - (Number(a.sizeBytes) || 0);
      if (sort === "name") return text(a.name).localeCompare(text(b.name));
      return text(b.createdAt).localeCompare(text(a.createdAt));
    });
    return sorted;
  }, [scoped, tab, sort]);

  const artifactDays = React.useMemo(() => {
    const map = new Map();
    for (const item of visible) {
      const key = utcDayKey(item.createdAt) ?? UNDATED;
      if (!map.has(key)) map.set(key, []);
      map.get(key).push(item);
    }
    const entries = [...map.entries()];
    // Undated last whichever way the dated groups run - it is the one group
    // whose position says nothing, so it should not take a meaningful one.
    entries.sort((a, b) => {
      if (a[0] === UNDATED) return 1;
      if (b[0] === UNDATED) return -1;
      return sort === "oldest" ? a[0].localeCompare(b[0]) : b[0].localeCompare(a[0]);
    });
    return entries.map(([key, list]) => ({
      key: key ?? "undated",
      label: dayLabel(key, now),
      items: list,
    }));
  }, [visible, sort, now]);

  /* conversations: Pinned, then Today / Yesterday / Earlier */
  const threadGroups = React.useMemo(() => {
    const sorted = scopedThreads.slice().sort((a, b) => {
      if (sort === "name") return text(a.title).localeCompare(text(b.title));
      if (sort === "oldest") return text(a.updatedAt).localeCompare(text(b.updatedAt));
      return text(b.updatedAt).localeCompare(text(a.updatedAt));
    });

    const nowDate = new Date(now);
    const today = nowDate.toISOString().slice(0, 10);
    const yesterday = new Date(nowDate.getTime() - 86_400_000).toISOString().slice(0, 10);

    const groups = [
      { key: "pinned", label: "Pinned", threads: [] },
      { key: "today", label: "Today", threads: [] },
      { key: "yesterday", label: "Yesterday", threads: [] },
      { key: "earlier", label: "Earlier", threads: [] },
    ];

    for (const t of sorted) {
      const day = utcDayKey(t.updatedAt);
      if (t.pinned) groups[0].threads.push(t);
      else if (day === today) groups[1].threads.push(t);
      else if (day === yesterday) groups[2].threads.push(t);
      // A thread with no readable `updatedAt` is not today's and not
      // yesterday's, which makes "Earlier" the honest place for it.
      else groups[3].threads.push(t);
    }
    return groups.filter((g) => g.threads.length > 0);
  }, [scopedThreads, sort, now]);

  const filtering = query.trim().length > 0 || agentId !== "all";

  function clearFilters() {
    setQuery("");
    setAgentId("all");
  }

  function startRename(thread) {
    setRenaming(thread);
    setRenameValue(thread.title);
  }

  function commitRename() {
    if (!renaming) return;
    const next = renameValue.trim();
    if (next && next !== renaming.title) renameThread(renaming.id, next);
    setRenaming(null);
  }

  // Deleting an artifact used to live here. There is nothing left to delete:
  // an artifact IS a message's attachment or a tool call's path, so removing
  // one would mean rewriting a transcript, which is the conversation's business
  // and not this screen's. Deleting the conversation still takes both.
  function confirmDeleteThread() {
    if (!deletingThread) return;
    deleteThread(deletingThread.id);
    setDeletingThread(null);
  }

  const detailAgent = openItem ? agentById.get(openItem.agentId) : null;
  const detailThread = openItem ? threadById.get(openItem.threadId) : null;
  const detailMeta = openItem ? LIBRARY_CATEGORY_META[openItem.category] : null;

  const showingConversations = tab === "conversation";

  return (
    <>
      <header className={cn("flex h-14 shrink-0 items-center", HEADER_GAP, GUTTER)}>
        <div className="flex min-w-0 items-baseline gap-2">
          <h1 className="text-sm font-medium text-foreground">Library</h1>
          <span className="truncate text-[11px] text-muted-foreground">
            {showingConversations
              ? `${counts.conversation} conversation${counts.conversation === 1 ? "" : "s"}`
              : `${visible.length} of ${items.length} artifacts`}
            {` · ${threads.length} chats archived`}
          </span>
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-2">
          <SearchInput
            size="sm"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onClear={() => setQuery("")}
            placeholder="Search the library"
            aria-label="Search the library"
            className="hidden w-52 md:flex"
          />
          <Select
            size="md"
            value={agentId}
            onChange={setAgentId}
            options={agentOptions}
            ariaLabel="Filter by agent"
            className="hidden w-36 lg:flex"
          />
          <Select
            size="md"
            value={sort}
            onChange={setSort}
            options={SORT_OPTIONS}
            ariaLabel="Sort the library"
            className="hidden w-36 xl:flex"
          />
          <Segmented
            size="xs"
            value={layout}
            onChange={setLayout}
            options={VIEW_OPTIONS}
            label="Switch between grid and list"
          />
        </div>
      </header>

      <div className={cn("shrink-0 pb-2", GUTTER)}>
        <Tabs
          variant="pill"
          value={tab}
          onChange={setTab}
          items={tabItems}
          ariaLabel="Library categories"
          idPrefix="library"
        />
      </div>

      <div className="flex min-h-0 flex-1">
        <ScrollArea className="min-w-0 flex-1">
          {/* Keyed on the tab and the layout, so a switch fades a new list in
              rather than reshuffling the old one's rows in place. */}
          <div
            key={`${tab}-${layout}`}
            className={cn("w-full animate-fade-in pb-12", GUTTER)}
            id={`library-panel-${tab}`}
            role="tabpanel"
            aria-labelledby={`library-tab-${tab}`}
          >
            {showingConversations ? (
              threadGroups.length === 0 ? (
                <EmptyState
                  icon={filtering ? SearchX : MessageCircle}
                  title={filtering ? "No conversations match" : "No conversations yet"}
                  description={
                    filtering
                      ? "Nothing here answers to that search and filter combination."
                      : "Start a chat with a teammate and it will be archived here."
                  }
                  action={
                    filtering ? (
                      <Button size="sm" onClick={clearFilters}>
                        Clear filters
                      </Button>
                    ) : null
                  }
                  className="mt-10"
                />
              ) : (
                threadGroups.map((group) => (
                  <section key={group.key}>
                    <h2 className={SECTION_HEADING}>{group.label}</h2>
                    <div className="flex flex-col gap-0.5">
                      {group.threads.map((thread) => (
                        <ThreadRow
                          key={thread.id}
                          thread={thread}
                          agent={agentById.get(thread.agentId)}
                          artifactCount={artifactsByThread.get(thread.id) ?? 0}
                          onOpen={() => openThread(thread.id)}
                          onPin={() => togglePinThread(thread.id)}
                          onRename={() => startRename(thread)}
                          onDelete={() => setDeletingThread(thread)}
                        />
                      ))}
                    </div>
                  </section>
                ))
              )
            ) : artifactDays.length === 0 ? (
              <EmptyState
                icon={filtering ? SearchX : FileSearch}
                title={
                  filtering
                    ? "Nothing matches that search"
                    : `No ${tab === "all" ? "artifacts" : LIBRARY_CATEGORY_META[tab].label.toLowerCase()} yet`
                }
                description={
                  filtering
                    ? "Try a shorter query, or widen the agent filter."
                    : "Files your teammates attach, capture or export land here automatically."
                }
                action={
                  filtering ? (
                    <Button size="sm" onClick={clearFilters}>
                      Clear filters
                    </Button>
                  ) : null
                }
                className="mt-10"
              />
            ) : (
              artifactDays.map((day) => (
                <section key={day.key}>
                  <h2 className={SECTION_HEADING}>{day.label}</h2>
                  {layout === "grid" ? (
                    <div className="grid grid-cols-1 gap-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5">
                      {day.items.map((item) => (
                        <ArtifactCard
                          key={item.id}
                          item={item}
                          agent={agentById.get(item.agentId)}
                          thread={threadById.get(item.threadId)}
                          onOpen={() => setOpenItem(item)}
                        />
                      ))}
                    </div>
                  ) : (
                    <div className="flex flex-col gap-0.5">
                      {day.items.map((item) => (
                        <ArtifactRow
                          key={item.id}
                          item={item}
                          agent={agentById.get(item.agentId)}
                          thread={threadById.get(item.threadId)}
                          onOpen={() => setOpenItem(item)}
                        />
                      ))}
                    </div>
                  )}
                </section>
              ))
            )}
          </div>
        </ScrollArea>
      </div>

      {/* ── artifact detail ─────────────────────────────────── */}
      <ArtifactDialog
        artifact={openItem}
        onOpenChange={() => setOpenItem(null)}
        facts={
          openItem
            ? [
                ["Kind", `${detailMeta.label.replace(/s$/, "")} · ${openItem.mime}`],
                openItem.sizeBytes != null ? ["Size", formatBytes(openItem.sizeBytes)] : null,
                openItem.origin === "written" ? ["Location", openItem.path] : null,
                ["Created", formatDateTime(openItem.createdAt)],
                ["From", detailAgent?.name ?? "You"],
                ["Conversation", detailThread?.title ?? "-"],
                ["Origin", SOURCE_LABEL[openItem.source] ?? openItem.source],
              ].filter(Boolean)
            : []
        }
        trailing={
          <Button
            variant="primary"
            size="sm"
            disabled={!detailThread}
            onClick={() => {
              const target = openItem.threadId;
              setOpenItem(null);
              openThread(target);
            }}
          >
            Open in chat
          </Button>
        }
      />

      {/* ── rename ──────────────────────────────────────────────────────── */}
      <Dialog open={Boolean(renaming)} onOpenChange={() => setRenaming(null)} size="sm">
        <DialogTitle>Rename conversation</DialogTitle>
        <DialogDescription>
          Only the title changes - the transcript and its artifacts stay put.
        </DialogDescription>
        <DialogBody>
          <Input
            autoFocus
            value={renameValue}
            onChange={(e) => setRenameValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                commitRename();
              }
            }}
            aria-label="Conversation title"
            placeholder="Conversation title"
          />
        </DialogBody>
        <DialogFooter>
          <Button variant="secondary" size="sm" onClick={() => setRenaming(null)}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" onClick={commitRename}>
            Save
          </Button>
        </DialogFooter>
      </Dialog>

      <ConfirmDialog
        open={Boolean(deletingThread)}
        onOpenChange={(next) => !next && setDeletingThread(null)}
        title="Delete this conversation?"
        description={
          deletingThread
            ? `“${deletingThread.title}” and its transcript will be removed. This cannot be undone.`
            : ""
        }
        confirmLabel="Delete"
        destructive
        onConfirm={confirmDeleteThread}
      />
    </>
  );
}
