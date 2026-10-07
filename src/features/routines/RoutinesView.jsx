import * as React from "react";
import { CalendarClock, Plus, Repeat } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { useNow } from "@/hooks/use-now";
import { utcDayKey } from "@/lib/datetime";
import { PREF, usePersistentState } from "@/lib/persist";
import { SCHEDULE_KIND_META, relativeTime } from "@/data";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Meter } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SearchInput } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Tabs } from "@/components/ui/tabs";
import { useToast } from "@/components/ui/toast";
import { RoutineRow } from "@/features/routines/RoutineRow";
import { RoutineDetail } from "@/features/routines/RoutineDetail";
import { NewRoutineDialog } from "@/features/routines/NewRoutineDialog";
import { absoluteTime } from "@/features/routines/run-status";
import { GUTTER, HEADER_GAP } from "@/components/layout/View";

const STATUS_TABS = [
  { value: "all", label: "All" },
  { value: "running", label: "Running" },
  { value: "scheduled", label: "Scheduled" },
  { value: "failing", label: "Failing" },
  { value: "disabled", label: "Disabled" },
];

// The order groups appear in. Anything unknown falls to the end.
const KIND_ORDER = ["cron", "interval", "trigger", "manual"];

function matchesStatus(routine, status) {
  const last = routine.lastRun;
  switch (status) {
    case "running":
      return last?.status === "running";
    case "scheduled":
      return Boolean(routine.enabled && routine.schedule?.nextRunAt);
    case "failing":
      return last?.status === "error";
    case "disabled":
      return !routine.enabled;
    default:
      return true;
  }
}

function StatCell({ label, value, hint, children, className }) {
  return (
    <div
      className={cn(
        "flex min-w-0 flex-1 flex-col gap-1 rounded-xl fill-whisper px-3.5 py-3",
        className
      )}
    >
      <span className="text-[11px] font-semibold text-muted-foreground">{label}</span>
      {value != null ? (
        <span className="truncate text-[13.5px] font-medium text-foreground">{value}</span>
      ) : null}
      {children}
      {hint ? <span className="truncate text-[11px] text-muted-foreground">{hint}</span> : null}
    </div>
  );
}

export function RoutinesView() {
  const { routines, agents, activeRoutineId, setActiveRoutineId } = useApp();
  const now = useNow();
  const { toast } = useToast();

  const [query, setQuery] = React.useState("");
  // a remembered agent that has since been deleted would pin this screen to zero results
  const [agentId, setAgentId] = usePersistentState(
    PREF.routinesAgent,
    "all",
    (v) => v === "all" || agents.some((b) => b.id === v)
  );
  const [status, setStatus] = usePersistentState(PREF.routinesStatus, "all", (v) =>
    STATUS_TABS.some((t) => t.value === v)
  );
  const [creating, setCreating] = React.useState(false);

  const agentOptions = React.useMemo(
    () => [
      { value: "all", label: "All agents" },
      ...agents.map((b) => ({ value: b.id, label: b.name })),
    ],
    [agents]
  );

  const filtered = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    return routines.filter((r) => {
      if (agentId !== "all" && r.agentId !== agentId) return false;
      if (!matchesStatus(r, status)) return false;
      if (!q) return true;
      return (
        (r.name ?? "").toLowerCase().includes(q) ||
        (r.description ?? "").toLowerCase().includes(q) ||
        (r.tags ?? []).some((t) => String(t).toLowerCase().includes(q))
      );
    });
  }, [routines, query, agentId, status]);

  const groups = React.useMemo(() => {
    const byKind = new Map();
    for (const routine of filtered) {
      const kind = routine.schedule?.kind ?? "manual";
      if (!byKind.has(kind)) byKind.set(kind, []);
      byKind.get(kind).push(routine);
    }
    return [...byKind.entries()]
      .sort((a, b) => {
        const ai = KIND_ORDER.indexOf(a[0]);
        const bi = KIND_ORDER.indexOf(b[0]);
        return (ai < 0 ? KIND_ORDER.length : ai) - (bi < 0 ? KIND_ORDER.length : bi);
      })
      .map(([kind, items]) => ({
        kind,
        label: SCHEDULE_KIND_META[kind]?.label ?? kind,
        items,
      }));
  }, [filtered]);

  const stats = React.useMemo(() => {
    const nowMs = new Date(now).getTime();
    const today = utcDayKey(now);

    // A routine whose `nextRunAt` is unreadable has no place on a "soonest
    // first" list - it would sort by the accident of its own text - so it is
    // not a candidate for the tile. It still renders in its group below.
    const upcoming = routines
      .filter((r) => r.enabled && utcDayKey(r.schedule?.nextRunAt))
      .sort((a, b) => a.schedule.nextRunAt.localeCompare(b.schedule.nextRunAt))[0];

    let done = 0;
    let ok = 0;
    let failures24h = 0;
    let runsToday = 0;

    for (const routine of routines) {
      for (const run of routine.runHistory ?? []) {
        if (run.status !== "running") {
          done += 1;
          if (run.status === "success") ok += 1;
        }
        const at = new Date(run.at).getTime();
        // Both windows are questions about when, and a run with no readable
        // `at` is not an answer to either - it counts towards the success rate
        // above, which only asks what happened.
        if (Number.isNaN(at)) continue;
        if (run.status === "error" && nowMs - at <= 86_400_000 && nowMs - at >= 0) failures24h += 1;
        if (utcDayKey(run.at) === today) runsToday += 1;
      }
    }

    return {
      upcoming,
      successRate: done ? Math.round((ok / done) * 100) : null,
      sampleSize: done,
      failures24h,
      runsToday,
    };
  }, [routines, now]);

  const activeCount = routines.filter((r) => r.enabled).length;
  const failingCount = routines.filter((r) => r.lastRun?.status === "error").length;

  const active = activeRoutineId ? routines.find((r) => r.id === activeRoutineId) : null;
  if (active) {
    return <RoutineDetail routineId={active.id} onBack={() => setActiveRoutineId(null)} />;
  }

  return (
    <>
      <header className={cn("flex h-14 shrink-0 items-center", HEADER_GAP, GUTTER)}>
        <div className="flex min-w-0 items-baseline gap-2">
          <h1 className="text-sm font-medium text-foreground">Routines</h1>
          <span className="truncate text-[11px] text-muted-foreground">
            {activeCount} active
            {failingCount ? ` · ${failingCount} failing` : ""}
          </span>
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-2">
          <SearchInput
            size="sm"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onClear={() => setQuery("")}
            placeholder="Search routines"
            aria-label="Search routines"
            className="hidden w-52 md:flex"
          />
          <Select
            size="sm"
            value={agentId}
            onChange={setAgentId}
            options={agentOptions}
            ariaLabel="Filter by agent"
            className="hidden w-36 lg:flex"
          />
          <Tabs
            variant="pill"
            value={status}
            onChange={setStatus}
            items={STATUS_TABS}
            ariaLabel="Filter routines by status"
            idPrefix="routines-status"
            className="hidden xl:flex"
          />
          <Button variant="primary" size="sm" onClick={() => setCreating(true)}>
            <Plus />
            New routine
          </Button>
        </div>
      </header>

      <ScrollArea className="flex-1">
        <div className={cn("w-full pb-10", GUTTER)}>
          {/* At a glance - flat cells, separated by whitespace, never a divider. */}
          <div className="flex flex-col gap-2 pt-1 sm:flex-row">
            <StatCell
              label="Next run"
              value={
                stats.upcoming ? relativeTime(stats.upcoming.schedule.nextRunAt) : "Nothing queued"
              }
              hint={stats.upcoming ? stats.upcoming.name : "No enabled routine has a next run"}
            />
            <StatCell label="Success rate">
              {stats.successRate == null ? (
                <span className="text-[13.5px] font-medium text-muted-foreground">No runs yet</span>
              ) : (
                <Meter
                  value={stats.successRate}
                  max={100}
                  tone={
                    stats.successRate >= 90
                      ? "success"
                      : stats.successRate >= 70
                        ? "warning"
                        : "danger"
                  }
                  label={`last ${stats.sampleSize} runs`}
                  className="mt-0.5"
                />
              )}
            </StatCell>
            <StatCell
              label="Failures 24h"
              value={String(stats.failures24h)}
              hint={stats.failures24h ? "Needs a look" : "Clean day"}
              className={stats.failures24h ? "text-destructive-ink" : undefined}
            />
            <StatCell label="Runs today" value={String(stats.runsToday)} hint={absoluteTime(now)} />
          </div>

          {/* Two empty states, because they are two different situations and
              telling them apart is the whole use of an empty state. A workspace
              with no routines in it was being told that nothing fit its search
              and filter combination, under a Clear filters button that cleared
              filters nobody had set - which reads as a screen that has lost the
              routine you just watched an agent create. */}
          {groups.length === 0 && routines.length === 0 ? (
            <EmptyState
              icon={Repeat}
              title="No routines yet"
              description="A routine is work an agent runs without being asked - a morning summary, a check every ten minutes, a weekly tidy. Agents can write their own, and you can write one here."
              action={
                <Button size="sm" onClick={() => setCreating(true)}>
                  <Plus />
                  New routine
                </Button>
              }
              className="mt-10"
            />
          ) : groups.length === 0 ? (
            <EmptyState
              icon={Repeat}
              title="No routines match"
              description={`None of your ${routines.length} routine${routines.length === 1 ? "" : "s"} fit that search and filter combination. Try widening one of them.`}
              action={
                <Button
                  size="sm"
                  onClick={() => {
                    setQuery("");
                    setAgentId("all");
                    setStatus("all");
                  }}
                >
                  Clear filters
                </Button>
              }
              className="mt-10"
            />
          ) : (
            groups.map((group) => (
              <section key={group.kind} className="mt-5">
                <h2 className="px-2.5 pb-1 text-[11px] font-semibold text-muted-foreground">
                  {group.label}
                  <span className="ml-1.5 font-normal opacity-70">{group.items.length}</span>
                </h2>
                <div className="flex flex-col gap-0.5">
                  {group.items.map((routine) => (
                    <RoutineRow
                      key={routine.id}
                      routine={routine}
                      onOpen={(id) => setActiveRoutineId(id)}
                    />
                  ))}
                </div>
              </section>
            ))
          )}

          {groups.length > 0 && stats.upcoming ? (
            <p className="mt-6 flex items-center gap-1.5 px-2.5 text-[11px] text-muted-foreground">
              <CalendarClock className="size-3.5" aria-hidden="true" />
              Next up: {stats.upcoming.name} - {absoluteTime(stats.upcoming.schedule.nextRunAt)}
            </p>
          ) : null}
        </div>
      </ScrollArea>

      <NewRoutineDialog
        open={creating}
        onOpenChange={setCreating}
        agentId={agentId === "all" ? undefined : agentId}
        onCreated={(id, name) =>
          toast({ variant: "success", title: "Routine created", description: name })
        }
      />
    </>
  );
}
