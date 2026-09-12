import * as React from "react";
import { ChevronDown, ChevronLeft, Copy, Download, MessageSquare, MoreHorizontal, Play, Trash2, Icon } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { formatDuration, relativeTime } from "@/data";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { DropdownMenu, MenuItem, MenuSeparator } from "@/components/ui/dropdown-menu";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Segmented } from "@/components/ui/segmented";
import { Select } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Tabs } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip } from "@/components/ui/tooltip";
import { useToast } from "@/components/ui/toast";
import { Markdown } from "@/features/chat/Markdown";
import { absoluteTime, runLabel, runTone } from "@/features/routines/run-status";
import { MODES } from "@shared/modes";
import { APPROVALS } from "@shared/approval";
import { routineApproval, routineMode } from "@shared/routines";
import {
  cronToHuman,
  nextRunLabel,
  useScheduleCheck,
} from "@/features/routines/schedule-check";


/** ISO to the value a datetime-local input wants, in local time. */
function toLocalInput(iso) {
  const ms = Date.parse(iso ?? "");
  if (!Number.isFinite(ms)) return "";
  const d = new Date(ms);
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** And back: what the input gives is local time with no zone, so ISO it here. */
function fromLocalInput(value) {
  const ms = Date.parse(value ?? "");
  return Number.isFinite(ms) ? new Date(ms).toISOString() : null;
}

const KIND_OPTIONS = [
  { value: "cron", label: "Scheduled" },
  { value: "interval", label: "Interval" },
  { value: "once", label: "Once" },
  { value: "trigger", label: "Triggered" },
  { value: "manual", label: "Manual" },
];

const INTERVALS = [
  { value: "PT5M", label: "Every 5 minutes" },
  { value: "PT10M", label: "Every 10 minutes" },
  { value: "PT20M", label: "Every 20 minutes" },
  { value: "PT30M", label: "Every 30 minutes" },
  { value: "PT1H", label: "Every hour" },
  { value: "PT4H", label: "Every 4 hours" },
  { value: "PT12H", label: "Every 12 hours" },
  { value: "PT24H", label: "Once a day" },
];

const TRIGGERS = [
  { value: "deploy.staging.succeeded", label: "On every staging deploy" },
  { value: "deploy.production.succeeded", label: "On every production deploy" },
  { value: "thread.message.received", label: "When a thread receives a message" },
  { value: "computer.status.changed", label: "When a computer changes status" },
  { value: "integration.webhook", label: "On an inbound webhook" },
];

function RunHistoryRow({ run }) {
  const [open, setOpen] = React.useState(false);
  const tone = runTone(run.status);
  return (
    <div className="rounded-xl transition-colors hover:fill-nav">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left outline-none focus-visible:fill-nav"
      >
        <span className={cn("size-2 shrink-0 rounded-full", tone.dot)} aria-hidden="true" />
        <span className="w-24 shrink-0 text-[11px] text-muted-foreground">{relativeTime(run.at)}</span>
        <span className="w-16 shrink-0 text-[11px] tabular-nums text-muted-foreground">
          {run.status === "running" ? "-" : formatDuration(run.durationMs)}
        </span>
        <span className={cn("min-w-0 flex-1 truncate text-[13px]", open ? "text-foreground" : "text-foreground/90")}>
          {run.summary}
        </span>
        <ChevronDown
          aria-hidden="true"
          className={cn(
            "size-3.5 shrink-0 text-muted-foreground transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
            open && "rotate-180"
          )}
        />
      </button>
      <Collapse open={open}>
        <div className="px-2.5 pb-2.5 pl-[4.6rem]">
          <p className="text-[11px] leading-relaxed text-muted-foreground">{run.summary}</p>
          <p className="mt-1 text-[11px] text-muted-foreground/70">
            {runLabel(run.status)} · {absoluteTime(run.at)}
          </p>
        </div>
      </Collapse>
    </div>
  );
}

export function RoutineDetail({ routineId, onBack }) {
  const { routines, agents, threads, openThread, toggleRoutine, runRoutine, saveRoutine, createRoutine, deleteRoutine } =
    useApp();
  const { toast } = useToast();

  const routine = routines.find((r) => r.id === routineId);

  const [draft, setDraft] = React.useState(routine?.markdown ?? "");
  const [tab, setTab] = React.useState("edit");
  const [confirmDelete, setConfirmDelete] = React.useState(false);

  // The cron expression is edited as a draft rather than written through on
  // every keystroke. It used to go straight onto the record, so "0 9 * * 1-"
  // was a saved schedule for as long as it took to type the next character,
  // and a typo left behind was a routine that never fired again with nothing
  // on screen to say so.
  const kind = routine?.schedule?.kind ?? "manual";
  const [cronDraft, setCronDraft] = React.useState(routine?.schedule?.expression ?? "");
  const check = useScheduleCheck(kind, cronDraft);

  // A different routine is a different document - never carry a draft across.
  React.useEffect(() => {
    setDraft(routine?.markdown ?? "");
    setCronDraft(routine?.schedule?.expression ?? "");
    setTab("edit");
  }, [routineId]); // eslint-disable-line react-hooks/exhaustive-deps

  // Written only once the parser has accepted it. An expression that cannot be
  // read is left in the field with the reason underneath, so nothing is lost
  // and nothing is stored that would never fire.
  React.useEffect(() => {
    if (!routine || kind !== "cron") return;
    if (!check.checked || !check.valid) return;
    if (cronDraft === routine.schedule?.expression) return;
    saveRoutine(routine.id, {
      schedule: {
        ...routine.schedule,
        expression: cronDraft,
        humanLabel: cronToHuman(cronDraft) ?? check.words ?? "Custom schedule",
      },
    });
  }, [check, cronDraft, kind, routine, saveRoutine]);

  if (!routine) {
    return (
      <div className="flex flex-1 items-center justify-center text-[13px] text-muted-foreground">
        That routine no longer exists.
      </div>
    );
  }

  const agent = agents.find((b) => b.id === routine.agentId);
  const status = routine.lastRun?.status ?? null;
  const running = status === "running";
  const tone = runTone(status);
  const dirty = draft !== (routine.markdown ?? "");
  const history = routine.runHistory ?? [];
  const spark = history.slice(0, 14).reverse();
  // Every run of this routine is written into one conversation, the same way
  // a chat is. It exists once the first run has started, and it is where the
  // tool calls and the reply actually are - the rows below hold one line each.
  const threadId = `routine-${routine.id}`;
  const conversation = threads.some((t) => t.id === threadId);

  function patchSchedule(patch) {
    saveRoutine(routine.id, { schedule: { ...routine.schedule, ...patch } });
  }

  function changeKind(nextKind) {
    if (nextKind === kind) return;
    const defaults = {
      cron: { expression: "0 9 * * 1-5", humanLabel: "Weekdays at 09:00 UTC" },
      interval: { expression: "PT1H", humanLabel: "Every hour" },
      trigger: { expression: TRIGGERS[0].value, humanLabel: TRIGGERS[0].label },
      manual: { expression: null, humanLabel: "Run manually" },
    }[nextKind];
    setCronDraft(defaults.expression ?? "");
    patchSchedule({ kind: nextKind, ...defaults, nextRunAt: nextKind === "manual" || nextKind === "trigger" ? null : routine.schedule?.nextRunAt ?? null });
  }

  function save() {
    saveRoutine(routine.id, { markdown: draft });
    toast({ title: "Playbook saved", description: routine.name, variant: "success" });
  }

  // A copy starts disabled so it cannot fire on the original's schedule by accident.
  function duplicate() {
    const name = `${routine.name} (copy)`;
    createRoutine({
      name,
      description: routine.description,
      agentId: routine.agentId,
      icon: routine.icon,
      markdown: draft,
      tags: routine.tags ?? [],
      enabled: false,
      schedule: { ...routine.schedule, nextRunAt: null },
    });
    toast({ title: "Duplicated", description: `${name} is open and disabled.`, variant: "success" });
  }

  return (
    <>
      <header className="flex h-14 shrink-0 items-center gap-2 px-4 sm:px-6">
        <IconButton size="lg" label="Back to routines" onClick={onBack}>
          <ChevronLeft />
        </IconButton>
        <span
          className="grid size-7 shrink-0 place-items-center rounded-full fill-control text-muted-foreground"
          aria-hidden="true"
        >
          <Icon name={routine.icon} className="size-3.5" />
        </span>
        <div className="flex min-w-0 items-baseline gap-2">
          <h1 className="truncate text-sm font-medium text-foreground">{routine.name}</h1>
          {agent ? <span className="shrink-0 text-[11px] text-muted-foreground">{agent.name}</span> : null}
          {status === "error" ? (
            <Badge variant="danger" size="sm">
              Failing
            </Badge>
          ) : null}
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-2.5">
          <Switch
            size="sm"
            checked={Boolean(routine.enabled)}
            onCheckedChange={() => toggleRoutine(routine.id)}
            label={`${routine.enabled ? "Disable" : "Enable"} ${routine.name}`}
          />
          {conversation ? (
            <Button variant="secondary" size="sm" onClick={() => openThread(threadId)}>
              <MessageSquare />
              {running ? "Watch" : "Open conversation"}
            </Button>
          ) : null}
          <Button variant="primary" size="sm" disabled={running} onClick={() => runRoutine(routine.id)}>
            {running ? <Spinner size="sm" /> : <Play />}
            {running ? "Running" : "Run now"}
          </Button>
          <DropdownMenu
            align="end"
            ariaLabel="Routine actions"
            trigger={
              <IconButton size="lg" label="Routine actions">
                <MoreHorizontal />
              </IconButton>
            }
          >
            <MenuItem icon={Copy} onSelect={duplicate}>
              Duplicate
            </MenuItem>
            <MenuItem
              icon={Download}
              onSelect={() => toast({ title: "Exported", description: `${routine.name}.md written to the workspace.` })}
            >
              Export
            </MenuItem>
            <MenuSeparator />
            <MenuItem icon={Trash2} danger onSelect={() => setConfirmDelete(true)}>
              Delete
            </MenuItem>
          </DropdownMenu>
        </div>
      </header>

      <ScrollArea className="flex-1">
        <div className="w-full px-4 pb-12 sm:px-6">
          {routine.description ? (
            <p className="pt-1 text-[13px] leading-relaxed text-muted-foreground">{routine.description}</p>
          ) : null}

          {/* Schedule */}
          <section className="mt-5">
            <h2 className="pb-2 text-[11px] font-semibold text-muted-foreground">Schedule</h2>
            <div className="flex flex-col gap-3 rounded-xl fill-whisper p-4">
              <Segmented
                size="xs"
                value={kind}
                onChange={changeKind}
                options={KIND_OPTIONS}
                label="Schedule kind"
              />

              {/* One box keyed on the kind, so changing it fades the fields
                  in as a set rather than swapping a label for a picker. */}
              <div key={kind} className="animate-fade-in">
              {kind === "cron" ? (
                <div className="flex flex-col gap-1.5">
                  <label htmlFor="routine-cron" className="text-[11px] text-foreground/90">
                    Cron expression
                  </label>
                  <Input
                    id="routine-cron"
                    size="xs"
                    value={cronDraft}
                    onChange={(e) => setCronDraft(e.target.value)}
                    spellCheck={false}
                    className="max-w-64 font-mono"
                    aria-invalid={check.checked && !check.valid ? true : undefined}
                    aria-describedby="routine-cron-reading"
                  />
                  {check.checked && !check.valid ? (
                    <p
                      id="routine-cron-reading"
                      className="text-[11px] leading-relaxed text-destructive-ink"
                    >
                      {check.reason} It has not been saved.
                    </p>
                  ) : (
                    <p id="routine-cron-reading" className="text-[11px] leading-relaxed text-muted-foreground">
                      {cronToHuman(cronDraft) ?? check.words ?? "Checking..."}
                      {nextRunLabel(check.nextRunAt) ? ` Next run ${nextRunLabel(check.nextRunAt)}.` : ""}
                    </p>
                  )}
                </div>
              ) : null}

              {kind === "interval" ? (
                <div className="flex flex-col gap-1.5">
                  <span className="text-[11px] text-foreground/90">Interval</span>
                  <Select
                    size="xs"
                    value={routine.schedule?.expression ?? "PT1H"}
                    onChange={(value) =>
                      patchSchedule({
                        expression: value,
                        humanLabel: INTERVALS.find((i) => i.value === value)?.label ?? "Custom interval",
                      })
                    }
                    options={INTERVALS}
                    ariaLabel="Interval"
                    className="max-w-64"
                  />
                </div>
              ) : null}

              {kind === "once" ? (
                <div className="flex flex-col gap-1.5">
                  <label htmlFor="routine-once" className="text-[11px] text-foreground/90">
                    Run at
                  </label>
                  <Input
                    id="routine-once"
                    size="xs"
                    type="datetime-local"
                    value={toLocalInput(routine.schedule?.expression)}
                    onChange={(e) => {
                      const iso = fromLocalInput(e.target.value);
                      if (!iso) return;
                      patchSchedule({ expression: iso, humanLabel: `Once, at ${new Date(iso).toLocaleString()}`, nextRunAt: iso });
                    }}
                    className="max-w-64"
                  />
                  <p className="text-[11px] leading-relaxed text-muted-foreground">
                    Runs one time at that moment, then stays here as a record. A conversation can
                    write one of these with the `later` tool.
                  </p>
                </div>
              ) : null}

              {kind === "trigger" ? (
                <div className="flex flex-col gap-1.5">
                  <span className="text-[11px] text-foreground/90">Trigger</span>
                  <Select
                    size="xs"
                    value={routine.schedule?.expression ?? TRIGGERS[0].value}
                    onChange={(value) =>
                      patchSchedule({
                        expression: value,
                        humanLabel: TRIGGERS.find((t) => t.value === value)?.label ?? "Custom trigger",
                      })
                    }
                    options={TRIGGERS}
                    ariaLabel="Trigger"
                    className="max-w-72"
                  />
                </div>
              ) : null}

              {kind === "manual" ? (
                <p className="text-[11px] leading-relaxed text-muted-foreground">
                  This routine only runs when someone presses Run now.
                </p>
              ) : null}
              </div>

              <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-[11px]">
                <span className="text-muted-foreground">Next run</span>
                <span className="font-medium text-foreground">
                  {routine.enabled && routine.schedule?.nextRunAt
                    ? relativeTime(routine.schedule.nextRunAt)
                    : "Not scheduled"}
                </span>
                {routine.schedule?.nextRunAt ? (
                  <span className="text-muted-foreground/70">
                    {absoluteTime(routine.schedule.nextRunAt)}
                  </span>
                ) : null}
              </div>
            </div>
          </section>

          {/* What the run holds, and whether it asks. The same two dials a
              conversation has, with the defaults a scheduled job needs: every
              tool, nobody asked. Shown so a person can see them and hold a
              routine back on purpose, because unattended "ask" means
              "refuse", and a refusal nobody chose is the run failing for no
              reason anyone can find. */}
          <section className="mt-5">
            <h2 className="pb-2 text-[11px] font-semibold text-muted-foreground">Permissions</h2>
            <div className="flex flex-col gap-3 rounded-xl fill-whisper p-4">
              <div className="flex flex-wrap gap-6">
                <div className="flex flex-col gap-1.5">
                  <span className="text-[11px] text-foreground/90">Mode</span>
                  <Select
                    size="xs"
                    value={routineMode(routine)}
                    onChange={(mode) => saveRoutine(routine.id, { mode })}
                    options={MODES.map((m) => ({ value: m.id, label: m.label, description: m.hint }))}
                    ariaLabel="Routine mode"
                    className="w-44"
                  />
                </div>
                <div className="flex flex-col gap-1.5">
                  <span className="text-[11px] text-foreground/90">Approval</span>
                  <Select
                    size="xs"
                    value={routineApproval(routine)}
                    onChange={(approval) => saveRoutine(routine.id, { approval })}
                    options={APPROVALS.map((a) => ({ value: a.id, label: a.label, description: a.hint }))}
                    ariaLabel="Routine approval"
                    className="w-44"
                  />
                </div>
              </div>
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                {routineApproval(routine) === "auto"
                  ? "Runs with nobody asked. Deny rules still hold, and emptying a folder is always refused unattended."
                  : "Nobody is there to answer, so anything this would ask about is refused instead. Set to Never ask if the routine keeps stopping short."}
              </p>
            </div>
          </section>

          {/* Playbook */}
          <section className="mt-5">
            <div className="flex items-center gap-2 pb-2">
              <h2 className="text-[11px] font-semibold text-muted-foreground">Playbook</h2>
              <Tabs
                variant="pill"
                size="sm"
                value={tab}
                onChange={setTab}
                items={[
                  { value: "edit", label: "Edit" },
                  { value: "preview", label: "Preview" },
                ]}
                ariaLabel="Playbook mode"
                idPrefix="routine-playbook"
                className="ml-1"
              />
              <div className="ml-auto flex items-center gap-2">
                {dirty ? (
                  <Tooltip content="This playbook has unsaved edits">
                    <span className="flex animate-fade-in items-center gap-1.5 text-[11px] text-warning-ink">
                      <span className="size-1.5 rounded-full bg-warning" aria-hidden="true" />
                      Unsaved
                    </span>
                  </Tooltip>
                ) : (
                  <span className="animate-fade-in text-[11px] text-muted-foreground">Saved</span>
                )}
                <Button size="sm" variant={dirty ? "primary" : "secondary"} disabled={!dirty} onClick={save}>
                  Save
                </Button>
              </div>
            </div>

            <div key={tab} className="animate-fade-in">
              {tab === "edit" ? (
                <Textarea
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  spellCheck={false}
                  aria-label="Playbook markdown"
                  className="min-h-[26rem] rounded-xl px-3.5 py-3 font-mono text-[12.5px] leading-relaxed"
                />
              ) : (
                <div className="min-h-[26rem] rounded-xl fill-whisper px-4 py-3">
                  <Markdown>{draft}</Markdown>
                </div>
              )}
            </div>
          </section>

          {/* Run history */}
          <section className="mt-5">
            <div className="flex items-baseline gap-2 pb-2">
              <h2 className="text-[11px] font-semibold text-muted-foreground">Run history</h2>
              {routine.lastRun ? (
                <span className={cn("text-[11px]", tone.ink)}>
                  Last {runLabel(status).toLowerCase()} · {relativeTime(routine.lastRun.at)}
                </span>
              ) : (
                <span className="text-[11px] text-muted-foreground">Never run</span>
              )}
            </div>

            {spark.length ? (
              <div className="flex h-7 items-end gap-1 px-2.5 pb-3" aria-hidden="true">
                {spark.map((run, i) => (
                  <span
                    key={`${run.at}-${i}`}
                    className={cn(
                      "h-full min-w-1.5 flex-1 rounded-sm",
                      runTone(run.status).bar,
                      run.status === "running" && "animate-soft-pulse"
                    )}
                    style={{ opacity: 0.45 + (0.55 * (i + 1)) / spark.length }}
                  />
                ))}
              </div>
            ) : null}

            {history.length ? (
              <div className="flex flex-col gap-0.5">
                {history.map((run, i) => (
                  <RunHistoryRow key={`${run.at}-${i}`} run={run} />
                ))}
              </div>
            ) : (
              <p className="px-2.5 text-[13px] text-muted-foreground">
                No runs yet. Press Run now and the first result will land here, with the full
                conversation a click away.
              </p>
            )}
          </section>
        </div>
      </ScrollArea>

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        destructive
        title={`Delete ${routine.name}?`}
        description="The playbook and its run history go with it. This cannot be undone."
        confirmLabel="Delete routine"
        onConfirm={() => {
          const name = routine.name;
          setConfirmDelete(false);
          deleteRoutine(routine.id);
          onBack?.();
          toast({ title: "Routine deleted", description: name, variant: "danger" });
        }}
      />
    </>
  );
}
