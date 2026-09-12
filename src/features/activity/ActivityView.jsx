import * as React from "react";
import { Download, ScrollText, ShieldAlert, SlidersHorizontal, Icon } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { useNow } from "@/hooks/use-now";
import { utcDayKey } from "@/lib/datetime";
import { ACTIVITY_CATEGORY_META, SEVERITY_META, relativeTime } from "@/data";
import { PREF, usePersistentState } from "@/lib/persist";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogTitle, DialogDescription, DialogBody } from "@/components/ui/dialog";
import { DropdownMenu, MenuItem, MenuSeparator, SubMenu } from "@/components/ui/dropdown-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SearchInput } from "@/components/ui/input";
import { Segmented } from "@/components/ui/segmented";
import { useToast } from "@/components/ui/toast";
import { ActivityRow, SEVERITY_BADGE } from "@/features/activity/ActivityRow";
import { exportFileName, toCsv } from "@/features/activity/export-csv";
import { absoluteTime } from "@/features/routines/run-status";
import { useWorkspace } from "@/lib/workspace";
import { GUTTER, HEADER_GAP } from "@/components/layout/View";

/* ── time window ──────────────────────────────────────────────────────────────
 * The range picker drives BOTH the feed and the chart, so what you count is
 * always what you can see. Buckets snap to a ladder of round durations and the
 * window snaps to a bucket edge, so a column never straddles two hours.
 * ─────────────────────────────────────────────────────────────────────────── */

const HOUR = 3_600_000;
const BUCKET_LADDER = [1, 2, 3, 6, 12, 24];
const MAX_BUCKETS = 18;

const RANGES = [
  { value: "24h", label: "24h", hours: 24 },
  { value: "3d", label: "3d", hours: 72 },
  { value: "7d", label: "7d", hours: 168 },
  { value: "all", label: "All", hours: null },
];

const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

/** The bucket an event with no readable timestamp lands in. `null` rather than
 *  a string, so it can never collide with a real day key. */
const UNDATED = null;

/** Today / Yesterday / an explicit date - never a bare ISO string. */
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

/** Axis tick: a date on a daily axis, a date + clock on anything finer. */
function tickLabel(ms, bucketHours) {
  const d = new Date(ms);
  // The window is arithmetic over timestamps, so one unreadable value upstream
  // arrives here as NaN. An axis with a blank tick is a bad axis; a blank window
  // is a bug report.
  if (Number.isNaN(d.getTime())) return "";
  const day = `${MONTHS[d.getUTCMonth()].slice(0, 3)} ${d.getUTCDate()}`;
  if (bucketHours >= 24) return day;
  return `${day} ${String(d.getUTCHours()).padStart(2, "0")}:00`;
}

/** Within a day group the day is already in the header, so the clock is the
 *  useful half of the timestamp. */
function clockLabel(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "-";
  return d.toISOString().slice(11, 16);
}

/* ── severity ─────────────────────────────────────────────────────────────── */

const SEVERITY_ORDER = ["info", "success", "warning", "danger"];

const SEVERITY_INK = {
  info: "text-foreground",
  success: "text-success-ink",
  warning: "text-warning-ink",
  danger: "text-destructive-ink",
};

/**
 * The chart collapses four severities into three bands. Info and success are
 * both "the system did its job", and painting them apart would spend two more
 * hues on a distinction the tiles beside the chart already make in numbers.
 * Warning and danger are the only things worth a colour from across the room.
 */
const STACK = ["normal", "warning", "danger"];

const STACK_TONE = {
  normal: "text-foreground/25",
  warning: "text-warning-ink",
  danger: "text-destructive-ink",
};

const STACK_DOT = {
  normal: "bg-foreground/25",
  warning: "bg-warning",
  danger: "bg-destructive",
};

const STACK_LABEL = { normal: "Normal", warning: "Warning", danger: "Blocked" };

function band(severity) {
  if (severity === "danger") return "danger";
  if (severity === "warning") return "warning";
  return "normal";
}

/* ── overview pieces ──────────────────────────────────────────────────────── */

/** A facet, not a card: it reads as a number and behaves as a filter. */
function FacetCell({ label, value, tone, selected, onClick, className }) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={cn(
        "flex min-w-0 items-baseline justify-between gap-2 rounded-lg px-2 py-1.5 text-left",
        "outline-none transition-colors duration-150 ease-out",
        "focus-visible:fill-nav",
        selected ? "fill-secondary" : "hover:fill-nav",
        className
      )}
    >
      <span
        className={cn(
          "min-w-0 truncate text-[11px]",
          selected ? "text-foreground" : "text-muted-foreground"
        )}
      >
        {label}
      </span>
      <span className={cn("shrink-0 text-[13.5px] font-medium tabular-nums", tone)}>{value}</span>
    </button>
  );
}

/**
 * Events over time - the one chart that answers a question you actually have on
 * this screen ("when did it get noisy, and was any of it bad?"). Hand-authored
 * SVG: a track column per bucket so empty hours still read as hours, and a
 * stack of at most three bands on top. `preserveAspectRatio="none"` lets it
 * stretch to any panel width; every label lives in HTML outside the SVG so
 * nothing typographic is ever scaled.
 */
function TimelineChart({ cells, max, count, label }) {
  const COL = 10;
  const BAR = 6.6;
  const PAD = (COL - BAR) / 2;

  return (
    <svg
      role="img"
      aria-label={label}
      viewBox={`0 0 ${Math.max(1, count) * COL} 100`}
      preserveAspectRatio="none"
      className="h-14 w-full sm:h-[4.25rem]"
    >
      {cells.map((cell, i) => {
        const x = i * COL + PAD;
        let cursor = 100;
        const parts = [];
        for (const key of STACK) {
          const n = cell[key];
          if (!n) continue;
          const h = Math.max(2.5, (n / max) * 100);
          cursor = Math.max(0, cursor - h);
          parts.push(
            <rect
              key={key}
              x={x}
              y={cursor}
              width={BAR}
              height={h}
              fill="currentColor"
              className={STACK_TONE[key]}
            />
          );
        }
        return (
          <g key={i}>
            <rect
              x={x}
              y="0"
              width={BAR}
              height="100"
              fill="currentColor"
              className="text-foreground/[0.045]"
            />
            {parts}
          </g>
        );
      })}
    </svg>
  );
}

/** A ranked proportion bar. Neutral by design - this ranks volume, not risk. */
function CategoryBar({ meta, count, total, selected, onClick }) {
  const pct = total ? Math.round((count / total) * 100) : 0;
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      aria-label={`${meta.label}: ${count} events, ${pct}% of this window`}
      className={cn(
        "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left outline-none",
        "transition-colors duration-150 ease-out",
        "focus-visible:fill-nav",
        selected ? "fill-secondary" : "hover:fill-nav"
      )}
    >
      <Icon name={meta.icon} className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="w-[4.5rem] shrink-0 truncate text-[12px] text-foreground">{meta.label}</span>
      <span className="block h-1 min-w-0 flex-1 rounded-full fill-secondary" aria-hidden="true">
        <span
          className="block h-1 rounded-full bg-foreground/30"
          style={{ width: `${Math.max(pct > 0 ? 6 : 0, pct)}%` }}
        />
      </span>
      <span className="w-5 shrink-0 text-right text-[11px] tabular-nums text-muted-foreground">
        {count}
      </span>
    </button>
  );
}

function SideHeading({ children }) {
  return (
    <h2 className="px-2 pt-4 pb-1.5 text-[11px] font-semibold text-muted-foreground first:pt-1">
      {children}
    </h2>
  );
}

/* ── the view ─────────────────────────────────────────────────────────────── */

export function ActivityView() {
  const { activity, agents, computers, setActiveAgentId, setView } = useApp();
  const now = useNow();
  const { toast } = useToast();
  const { client, native } = useWorkspace();

  // The search box is deliberately NOT persisted: a query restored on launch
  // looks like a broken feed, not a remembered preference.
  const [query, setQuery] = React.useState("");
  const [openEvent, setOpenEvent] = React.useState(null);

  const [category, setCategory] = usePersistentState(
    PREF.activityCategory,
    "all",
    (v) => v === "all" || Object.prototype.hasOwnProperty.call(ACTIVITY_CATEGORY_META, v)
  );
  const [severity, setSeverity] = usePersistentState(
    PREF.activitySeverity,
    "all",
    (v) => v === "all" || Object.prototype.hasOwnProperty.call(SEVERITY_META, v)
  );
  const [agentId, setAgentId] = usePersistentState(
    PREF.activityAgent,
    "all",
    (v) => v === "all" || v === "none" || agents.some((b) => b.id === v)
  );
  const [range, setRange] = usePersistentState(
    PREF.activityRange,
    "all",
    (v) => RANGES.some((r) => r.value === v)
  );

  const nowMs = React.useMemo(() => new Date(now).getTime(), [now]);
  const rangeHours = RANGES.find((r) => r.value === range)?.hours ?? null;

  /** Bucket size, then a window snapped outward to whole buckets. */
  const win = React.useMemo(() => {
    let earliest = nowMs;
    for (const e of activity) {
      const t = new Date(e.at).getTime();
      if (!Number.isNaN(t) && t < earliest) earliest = t;
    }
    const startRaw = rangeHours == null ? earliest : nowMs - rangeHours * HOUR;
    const spanHours = Math.max(6, Math.ceil((nowMs - startRaw) / HOUR));
    const bucketHours = BUCKET_LADDER.find((h) => spanHours / h <= MAX_BUCKETS) ?? 24;
    const bucketMs = bucketHours * HOUR;
    const end = Math.ceil(nowMs / bucketMs) * bucketMs;
    const start = Math.floor(startRaw / bucketMs) * bucketMs;
    return { start, end, bucketMs, bucketHours, count: Math.max(1, Math.round((end - start) / bucketMs)) };
  }, [activity, nowMs, rangeHours]);

  /**
   * A time filter cannot answer for an event that has no time, so it does not
   * pretend to: with a range chosen, an undated event is out of it; with "All"
   * chosen, nothing has been excluded and hiding it would be the filter lying.
   * Either way it stays in the log rather than disappearing from the one screen
   * that would show the user their record is malformed.
   */
  const inRange = React.useMemo(
    () =>
      activity.filter((e) => {
        const t = new Date(e.at).getTime();
        if (Number.isNaN(t)) return rangeHours == null;
        return t >= win.start && t <= win.end;
      }),
    [activity, win, rangeHours]
  );

  /**
   * `scoped` ignores the severity filter on purpose: the tiles are the severity
   * control, so they have to show what picking each one would give you. The
   * chart shares that scope so its bands stay a legend for the tiles.
   */
  const scoped = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    return inRange.filter((e) => {
      if (category !== "all" && e.category !== category) return false;
      if (agentId === "none" && e.agentId) return false;
      if (agentId !== "all" && agentId !== "none" && e.agentId !== agentId) return false;
      if (!q) return true;
      return (e.title ?? "").toLowerCase().includes(q) || (e.detail ?? "").toLowerCase().includes(q);
    });
  }, [inRange, query, category, agentId]);

  const filtered = React.useMemo(
    () => (severity === "all" ? scoped : scoped.filter((e) => e.severity === severity)),
    [scoped, severity]
  );

  const days = React.useMemo(() => {
    const map = new Map();
    for (const event of filtered) {
      const key = utcDayKey(event.at) ?? UNDATED;
      if (!map.has(key)) map.set(key, []);
      map.get(key).push(event);
    }
    return [...map.entries()]
      // Newest day first, with the undated group last - it is the one group
      // whose position carries no meaning, so it should not displace one whose
      // position does.
      .sort((a, b) => {
        if (a[0] === UNDATED) return 1;
        if (b[0] === UNDATED) return -1;
        return b[0].localeCompare(a[0]);
      })
      .map(([key, events]) => ({ key: key ?? "undated", label: dayLabel(key, now), events }));
  }, [filtered, now]);

  const counts = React.useMemo(() => {
    const base = { info: 0, success: 0, warning: 0, danger: 0 };
    for (const e of scoped) if (base[e.severity] != null) base[e.severity] += 1;
    return base;
  }, [scoped]);

  const chart = React.useMemo(() => {
    const cells = Array.from({ length: win.count }, () => ({ normal: 0, warning: 0, danger: 0, total: 0 }));
    for (const e of scoped) {
      const i = Math.floor((new Date(e.at).getTime() - win.start) / win.bucketMs);
      const cell = cells[Math.min(win.count - 1, Math.max(0, i))];
      if (!cell) continue;
      cell[band(e.severity)] += 1;
      cell.total += 1;
    }
    return { cells, max: Math.max(1, ...cells.map((c) => c.total)) };
  }, [scoped, win]);

  const chartLabel = React.useMemo(() => {
    const unit = win.bucketHours >= 24 ? "day" : `${win.bucketHours}-hour block`;
    const series = chart.cells.map((c) => c.total).join(", ");
    return (
      `Events over time. ${scoped.length} events per ${unit}, from ` +
      `${tickLabel(win.start, win.bucketHours)} to ${tickLabel(win.end, win.bucketHours)} UTC. ` +
      `Busiest block ${chart.max}. Counts per block: ${series}. ` +
      `${counts.warning} warning and ${counts.danger} blocked in this window.`
    );
  }, [chart, counts, scoped.length, win]);

  const categoryRanked = React.useMemo(() => {
    const tally = new Map();
    for (const e of scoped) tally.set(e.category, (tally.get(e.category) ?? 0) + 1);
    return [...tally.entries()]
      .sort((a, b) => b[1] - a[1] || String(a[0]).localeCompare(String(b[0])))
      .map(([key, count]) => ({
        key,
        count,
        meta: ACTIVITY_CATEGORY_META[key] ?? { label: key, icon: "Circle" },
      }));
  }, [scoped]);

  const topAgents = React.useMemo(() => {
    const tally = new Map();
    for (const e of scoped) {
      if (!e.agentId) continue;
      tally.set(e.agentId, (tally.get(e.agentId) ?? 0) + 1);
    }
    return [...tally.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 4)
      .map(([id, count]) => ({ agent: agents.find((b) => b.id === id), count }))
      .filter((entry) => entry.agent);
  }, [scoped, agents]);

  const needsAttention = React.useMemo(
    () => scoped.filter((e) => e.severity === "danger" || e.severity === "warning").slice(0, 4),
    [scoped]
  );

  const categoryOptions = React.useMemo(
    () => Object.entries(ACTIVITY_CATEGORY_META).map(([value, meta]) => ({ value, ...meta })),
    []
  );

  const menuFilterCount = (category !== "all" ? 1 : 0) + (agentId !== "all" ? 1 : 0);

  const categoryLabel =
    category === "all" ? "All" : (ACTIVITY_CATEGORY_META[category]?.label ?? category);
  const agentLabel =
    agentId === "all"
      ? "All"
      : agentId === "none"
        ? "No agent"
        : (agents.find((b) => b.id === agentId)?.name ?? "Unknown");

  function openAgent(id) {
    if (!id) return;
    setActiveAgentId(id);
    setView("agents");
  }

  /**
   * Write what is on screen to a file.
   *
   * The filtered list rather than the whole log, because the filters are how a
   * person says what they wanted an export of. In the app it lands in the
   * workspace's own `files/exports`, which needs no picker and no new IPC; in a
   * browser preview there is no folder at all, so it goes through the browser's
   * own download instead of claiming a file was written somewhere.
   */
  async function exportLog() {
    const csv = toCsv(filtered, agents);
    const name = exportFileName(new Date());

    if (!native) {
      const href = URL.createObjectURL(new Blob([csv], { type: "text/csv" }));
      const anchor = document.createElement("a");
      anchor.href = href;
      anchor.download = name;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      URL.revokeObjectURL(href);
      toast({ title: "Downloaded", description: `${filtered.length} events as ${name}.` });
      return;
    }

    const relPath = `files/exports/${name}`;
    try {
      await client.writeFile(relPath, csv);
      toast({
        title: "Exported",
        description: `${filtered.length} events written to ${relPath}.`,
        variant: "success",
        action: { label: "Show", onClick: () => client.reveal(relPath).catch(() => {}) },
      });
    } catch (error) {
      toast({
        title: "Could not export the log",
        description: error?.message ?? `${relPath} could not be written.`,
        variant: "warning",
      });
    }
  }

  function clearAll() {
    setQuery("");
    setCategory("all");
    setSeverity("all");
    setAgentId("all");
    setRange("all");
  }

  const eventAgent = openEvent?.agentId ? agents.find((b) => b.id === openEvent.agentId) : null;
  const eventComputer = openEvent?.computerId
    ? computers.find((c) => c.id === openEvent.computerId)
    : null;
  const eventSeverity = openEvent ? SEVERITY_META[openEvent.severity] ?? SEVERITY_META.info : null;
  const eventCategory = openEvent
    ? ACTIVITY_CATEGORY_META[openEvent.category] ?? { label: openEvent.category, icon: "Circle" }
    : null;

  return (
    <>
      {/* ── fixed header ─────────────────────────────────────────────────── */}
      <header className={cn("flex h-14 shrink-0 items-center", HEADER_GAP, GUTTER)}>
        <div className="flex min-w-0 items-baseline gap-2">
          <h1 className="text-sm font-medium text-foreground">Activity</h1>
          <span className="hidden truncate text-[11px] tabular-nums text-muted-foreground sm:inline">
            {filtered.length} of {activity.length} events
          </span>
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-2">
          <SearchInput
            size="sm"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onClear={() => setQuery("")}
            placeholder="Search activity"
            aria-label="Search activity"
            className="w-36 lg:w-52"
          />

          <DropdownMenu
            align="end"
            ariaLabel="Activity filters"
            trigger={
              <Button variant="ghost" size="sm" aria-label="Filters">
                <SlidersHorizontal />
                <span className="hidden md:inline">Filters</span>
                {menuFilterCount ? (
                  <Badge variant="neutral" size="sm">
                    {menuFilterCount}
                  </Badge>
                ) : null}
              </Button>
            }
          >
            <SubMenu label="Category" value={categoryLabel}>
              <MenuItem checked={category === "all"} onSelect={() => setCategory("all")}>
                All categories
              </MenuItem>
              <MenuSeparator />
              {categoryOptions.map((opt) => (
                <MenuItem
                  key={opt.value}
                  icon={<Icon name={opt.icon} />}
                  checked={category === opt.value}
                  onSelect={() => setCategory(opt.value)}
                >
                  {opt.label}
                </MenuItem>
              ))}
            </SubMenu>

            <SubMenu label="Agent" value={agentLabel}>
              <MenuItem checked={agentId === "all"} onSelect={() => setAgentId("all")}>
                All agents
              </MenuItem>
              <MenuItem checked={agentId === "none"} onSelect={() => setAgentId("none")}>
                No agent
              </MenuItem>
              <MenuSeparator />
              {agents.map((b) => (
                <MenuItem key={b.id} checked={agentId === b.id} onSelect={() => setAgentId(b.id)}>
                  {b.name}
                </MenuItem>
              ))}
            </SubMenu>

            <MenuSeparator />
            <MenuItem
              disabled={menuFilterCount === 0}
              onSelect={() => {
                setCategory("all");
                setAgentId("all");
              }}
            >
              Reset these filters
            </MenuItem>
          </DropdownMenu>

          <Button
            variant="ghost"
            size="sm"
            aria-label="Export log"
            disabled={!filtered.length}
            onClick={exportLog}
          >
            <Download />
            <span className="hidden lg:inline">Export log</span>
          </Button>
        </div>
      </header>

      {/* ── fixed overview strip ─────────────────────────────────────────── */}
      <div className={cn("shrink-0 pb-3", GUTTER)}>
        <div className="flex flex-wrap items-stretch gap-x-4 gap-y-3 rounded-2xl fill-whisper p-3.5">
          {/* severity facets - the numbers, and the severity filter */}
          <div className="flex w-full min-w-0 flex-col gap-1 sm:w-[13rem] sm:shrink-0">
            <FacetCell
              label="All events"
              value={scoped.length}
              tone="text-foreground"
              selected={severity === "all"}
              onClick={() => setSeverity("all")}
            />
            <div className="grid grid-cols-2 gap-1">
              {SEVERITY_ORDER.map((key) => (
                <FacetCell
                  key={key}
                  label={SEVERITY_META[key].label}
                  value={counts[key] ?? 0}
                  tone={SEVERITY_INK[key]}
                  selected={severity === key}
                  onClick={() => setSeverity(severity === key ? "all" : key)}
                />
              ))}
            </div>
          </div>

          {/* events over time */}
          <div className="flex min-w-[16rem] flex-1 flex-col gap-1.5">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
              <h2 className="text-[11px] font-semibold text-muted-foreground">Events over time</h2>
              <Segmented
                size="xs"
                value={range}
                onChange={setRange}
                options={RANGES}
                label="Time range"
                className="ml-auto"
              />
            </div>

            <TimelineChart
              cells={chart.cells}
              max={chart.max}
              count={win.count}
              label={chartLabel}
            />

            <div className="flex items-center gap-3 text-[10px] text-muted-foreground">
              <span className="shrink-0 tabular-nums">{tickLabel(win.start, win.bucketHours)}</span>
              <span className="flex min-w-0 flex-1 flex-wrap items-center justify-center gap-x-2.5 gap-y-1">
                {STACK.map((key) => (
                  <span key={key} className="inline-flex shrink-0 items-center gap-1">
                    <span
                      aria-hidden="true"
                      className={cn("size-1.5 rounded-full", STACK_DOT[key])}
                    />
                    {STACK_LABEL[key]}
                  </span>
                ))}
              </span>
              <span className="shrink-0 tabular-nums">now</span>
            </div>
          </div>
        </div>
      </div>

      {/* ── scrolling body ───────────────────────────────────────────────── */}
      <div className="flex min-h-0 flex-1">
        <ScrollArea className="min-w-0 flex-1">
          <div className={cn("w-full pb-10", GUTTER)}>
            {days.length === 0 ? (
              <EmptyState
                icon={ScrollText}
                title="Nothing logged here"
                description="No events match that search, range and filter combination."
                action={
                  <Button size="sm" onClick={clearAll}>
                    Clear filters
                  </Button>
                }
                className="mt-10"
              />
            ) : (
              days.map((day) => (
                <section key={day.key}>
                  <h2 className="sticky top-0 z-10 flex items-baseline gap-2 bg-background px-2.5 pt-4 pb-1.5 text-[11px] font-semibold text-muted-foreground">
                    {day.label}
                    <span className="font-normal tabular-nums text-muted-foreground/70">
                      {day.events.length}
                    </span>
                  </h2>

                  {/* A spine, not a border: the row icons sit on it like beads,
                      which is what makes a day read as one run of time.

                      Through the middle of the icons, by construction rather
                      than by eye. A row is ROW - pulled out by -mx-3 and padded
                      back by px-3 - so its icon starts at this box's left edge,
                      and the icon is size-8; the spine sits at half of that.
                      It was at 1.625rem, a number that matched an older row
                      and has been a hair to one side of every icon since. */}
                  <div className="relative">
                    <span
                      aria-hidden="true"
                      className="pointer-events-none absolute top-3 bottom-3 left-4 w-px bg-foreground/10"
                    />
                    <div className="relative flex flex-col gap-0.5">
                      {day.events.map((event) => (
                        <ActivityRow
                          key={event.id}
                          event={event}
                          timeLabel={clockLabel(event.at)}
                          onClick={() => setOpenEvent(event)}
                        />
                      ))}
                    </div>
                  </div>
                </section>
              ))
            )}
          </div>
        </ScrollArea>

        {/* Supplementary, never exclusive: every filter it offers is also in the
            header menu, so hiding it below xl costs no capability. */}
        <aside className="hidden w-[17rem] shrink-0 pr-4 sm:pr-6 xl:block">
          <ScrollArea className="h-full pb-10">
            <SideHeading>By category</SideHeading>
            <div className="flex flex-col gap-0.5">
              {categoryRanked.length ? (
                categoryRanked.map((row) => (
                  <CategoryBar
                    key={row.key}
                    meta={row.meta}
                    count={row.count}
                    total={scoped.length}
                    selected={category === row.key}
                    onClick={() => setCategory(category === row.key ? "all" : row.key)}
                  />
                ))
              ) : (
                <p className="px-2 text-[11px] text-muted-foreground">Nothing in this range.</p>
              )}
            </div>

            <SideHeading>Most active</SideHeading>
            <div className="flex flex-col gap-0.5">
              {topAgents.length ? (
                topAgents.map(({ agent, count }) => (
                  <button
                    key={agent.id}
                    type="button"
                    onClick={() => openAgent(agent.id)}
                    className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-left outline-none transition-colors duration-150 ease-out hover:fill-nav focus-visible:fill-nav"
                  >
                    <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                      {agent.name}
                    </span>
                    <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
                      {count}
                    </span>
                  </button>
                ))
              ) : (
                <p className="px-2 text-[11px] text-muted-foreground">No agent activity here.</p>
              )}
            </div>

            <SideHeading>Needs attention</SideHeading>
            <div className="flex flex-col gap-0.5">
              {needsAttention.length ? (
                needsAttention.map((event) => (
                  <button
                    key={event.id}
                    type="button"
                    onClick={() => setOpenEvent(event)}
                    className="flex flex-col gap-0.5 rounded-lg px-2 py-1.5 text-left outline-none transition-colors duration-150 ease-out hover:fill-nav focus-visible:fill-nav"
                  >
                    <span
                      className={cn(
                        "line-clamp-2 text-[12px] leading-snug",
                        SEVERITY_INK[event.severity] ?? "text-foreground"
                      )}
                    >
                      {event.title}
                    </span>
                    <span className="truncate text-[11px] text-muted-foreground">
                      {relativeTime(event.at)}
                      {event.agentId ? ` · ${agents.find((b) => b.id === event.agentId)?.name ?? ""}` : ""}
                    </span>
                  </button>
                ))
              ) : (
                <p className="px-2 text-[11px] text-muted-foreground">
                  Nothing needs a human right now.
                </p>
              )}
            </div>
          </ScrollArea>
        </aside>
      </div>

      <Dialog open={Boolean(openEvent)} onOpenChange={() => setOpenEvent(null)} size="md">
        {openEvent ? (
          <>
            <DialogTitle>{openEvent.title}</DialogTitle>
            <DialogDescription>{openEvent.detail}</DialogDescription>
            <DialogBody>
              <dl className="flex flex-col gap-2 rounded-xl fill-whisper p-4 text-[13px]">
                {[
                  ["When", absoluteTime(openEvent.at)],
                  ["Actor", openEvent.actor],
                  ["Agent", eventAgent?.name ?? "-"],
                  ["Computer", eventComputer?.name ?? "-"],
                  ["Target", openEvent.target ?? "-"],
                ].map(([label, value]) => (
                  <div key={label} className="flex items-baseline gap-3">
                    <dt className="w-20 shrink-0 text-[11px] text-muted-foreground">{label}</dt>
                    <dd className="min-w-0 flex-1 break-words text-foreground/90">{value}</dd>
                  </div>
                ))}
                <div className="flex items-center gap-3">
                  <dt className="w-20 shrink-0 text-[11px] text-muted-foreground">Classified</dt>
                  <dd className="flex flex-wrap items-center gap-1.5">
                    <Badge variant="neutral" size="sm">
                      <Icon name={eventCategory.icon} className="size-3" />
                      {eventCategory.label}
                    </Badge>
                    <Badge variant={SEVERITY_BADGE[openEvent.severity] ?? "neutral"} size="sm">
                      {eventSeverity.label}
                    </Badge>
                  </dd>
                </div>
              </dl>

              {openEvent.category === "permission" ? (
                <div className="mt-4 flex items-center gap-2">
                  <ShieldAlert className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <p className="min-w-0 flex-1 text-[11px] leading-relaxed text-muted-foreground">
                    This event came from the permission policy.
                  </p>
                  <Button
                    variant="primary"
                    size="sm"
                    disabled={!openEvent.agentId}
                    onClick={() => {
                      const id = openEvent.agentId;
                      setOpenEvent(null);
                      openAgent(id);
                    }}
                  >
                    Review permissions
                  </Button>
                </div>
              ) : null}
            </DialogBody>
          </>
        ) : null}
      </Dialog>
    </>
  );
}
