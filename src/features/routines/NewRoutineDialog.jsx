import * as React from "react";
import { Icon } from "@/components/icons";
import { useApp } from "@/lib/store";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Segmented } from "@/components/ui/segmented";
import { Select } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { cronToHuman, nextRunLabel, useScheduleCheck } from "@/features/routines/schedule-check";

const KIND_OPTIONS = [
  { value: "cron", label: "Scheduled" },
  { value: "interval", label: "Interval" },
  { value: "once", label: "Once" },
  { value: "trigger", label: "Triggered" },
  { value: "manual", label: "Manual" },
];

const INTERVALS = [
  { value: "PT5M", label: "Every 5 minutes" },
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

// Names, not components, because the chosen one is persisted with the routine
// and read back through `<Icon name>`. That makes a typo invisible: the picker
// drew a plain dot where the seventh choice should have been, because it asked
// for "GitPullRequest" and the set has never had one - `GitBranch` is the git
// glyph. `icons.test.js` now fails on a name the set cannot resolve.
const ICONS = [
  "Repeat",
  "Sunrise",
  "Binoculars",
  "ShieldCheck",
  "FlaskConical",
  "Inbox",
  "GitBranch",
  "Database",
];

const STARTER = `# Purpose
One sentence on what this routine is for and who reads its output.

## Inputs
- Where the facts come from

## Steps
1. First thing to do
2. Then this

## Guardrails
- What this routine must never do

## Success criteria
- How you know the run was good`;

/** The schedule expression control changes shape with the kind - same as the detail view. */
function ScheduleControl({ kind, expression, onChange, check }) {
  if (kind === "cron") {
    return (
      <Field label="Cron expression">
        <Input
          size="sm"
          value={expression}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
          placeholder="0 9 * * 1-5"
          aria-label="Cron expression"
          aria-invalid={check.checked && !check.valid ? true : undefined}
          aria-describedby="new-routine-cron-reading"
          className="font-mono"
        />
        <ScheduleReading id="new-routine-cron-reading" expression={expression} check={check} />
      </Field>
    );
  }
  if (kind === "interval") {
    return (
      <Field label="Interval">
        <Select
          size="md"
          value={expression}
          onChange={onChange}
          options={INTERVALS}
          ariaLabel="Interval"
        />
      </Field>
    );
  }
  if (kind === "once") {
    return (
      <Field label="Run at">
        <Input
          size="sm"
          type="datetime-local"
          value={toLocalInput(expression)}
          onChange={(e) => {
            const ms = Date.parse(e.target.value);
            if (Number.isFinite(ms)) onChange(new Date(ms).toISOString());
          }}
          aria-label="Run at"
        />
      </Field>
    );
  }
  if (kind === "trigger") {
    return (
      <Field label="Trigger">
        <Select
          size="md"
          value={expression}
          onChange={onChange}
          options={TRIGGERS}
          ariaLabel="Trigger"
        />
      </Field>
    );
  }
  return (
    <p className="text-[11px] leading-relaxed text-muted-foreground">
      This routine only runs when someone presses Run now.
    </p>
  );
}

/**
 * What the schedule means, or why it cannot be saved.
 *
 * One line under the field rather than a dialog on submit: a schedule is a
 * thing people get right by seeing it read back, and the reason a bad one is
 * refused should be visible while it is still being typed.
 */
function ScheduleReading({ id, expression, check }) {
  // Keyed on the verdict, so a reading turning into a refusal fades rather
  // than recolours in place.
  if (check.checked && !check.valid) {
    return (
      <p
        key="invalid"
        id={id}
        className="animate-fade-in text-[11px] leading-relaxed text-destructive-ink"
      >
        {check.reason}
      </p>
    );
  }
  const words = cronToHuman(expression) ?? check.words;
  const next = nextRunLabel(check.nextRunAt);
  return (
    <p
      key="reading"
      id={id}
      className="animate-fade-in text-[11px] leading-relaxed text-muted-foreground"
    >
      {words ?? "Checking..."}
      {next ? ` Next run ${next}.` : ""}
    </p>
  );
}

const DEFAULT_EXPRESSION = {
  cron: "0 9 * * 1-5",
  interval: "PT1H",
  trigger: TRIGGERS[0].value,
  // An hour from now, rounded to the minute: a time to edit, not a blank.
  once: new Date(Math.ceil(Date.now() / 60_000) * 60_000 + 60 * 60_000).toISOString(),
  manual: null,
};

function toLocalInput(iso) {
  const ms = Date.parse(iso ?? "");
  if (!Number.isFinite(ms)) return "";
  const d = new Date(ms);
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function humanLabel(kind, expression) {
  if (kind === "interval")
    return INTERVALS.find((i) => i.value === expression)?.label ?? "Custom interval";
  if (kind === "trigger")
    return TRIGGERS.find((t) => t.value === expression)?.label ?? "Custom trigger";
  if (kind === "cron") return `Cron · ${expression}`;
  if (kind === "once") return `Once, at ${new Date(expression).toLocaleString()}`;
  return "Run manually";
}

export function NewRoutineDialog({ open, onOpenChange, agentId, onCreated }) {
  const { agents, createRoutine } = useApp();

  const [name, setName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [agent, setAgent] = React.useState(agentId ?? agents[0]?.id ?? null);
  const [icon, setIcon] = React.useState(ICONS[0]);
  const [kind, setKind] = React.useState("cron");
  const [expression, setExpression] = React.useState(DEFAULT_EXPRESSION.cron);
  const [markdown, setMarkdown] = React.useState(STARTER);

  // Checked by the same parser the scheduler fires on, so what this dialog
  // accepts and what will actually run are the same set of expressions.
  const check = useScheduleCheck(kind, expression);

  // a dialog that keeps yesterday's draft reads as a bug
  React.useEffect(() => {
    if (!open) return;
    setName("");
    setDescription("");
    setAgent(agentId ?? agents[0]?.id ?? null);
    setIcon(ICONS[0]);
    setKind("cron");
    setExpression(DEFAULT_EXPRESSION.cron);
    setMarkdown(STARTER);
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  const agentOptions = agents.map((b) => ({ value: b.id, label: b.name, description: b.role }));
  const iconOptions = ICONS.map((value) => ({
    value,
    label: value,
    icon: <Icon name={value} />,
  }));

  const trimmed = name.trim();

  function changeKind(next) {
    setKind(next);
    setExpression(DEFAULT_EXPRESSION[next]);
  }

  function submit() {
    // The button is disabled for an unreadable schedule, and this is here for
    // the keyboard path into the same function. Storing one anyway is what
    // produced routines that sat in the list looking scheduled and never ran.
    if (!check.valid) return;
    const id = createRoutine({
      name: trimmed,
      description: description.trim(),
      agentId: agent,
      icon,
      markdown,
      schedule: {
        kind,
        expression,
        humanLabel: humanLabel(kind, expression),
        nextRunAt: null,
      },
    });
    onOpenChange(false);
    onCreated?.(id, trimmed);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange} size="lg">
      <DialogTitle>New routine</DialogTitle>
      <DialogDescription>
        A routine is a Markdown playbook a teammate runs on a schedule. You can edit every part of
        it afterwards.
      </DialogDescription>

      <DialogBody>
        <ScrollArea className="max-h-[min(30rem,calc(100dvh-16rem))] -mx-1 px-1">
          <div className="flex flex-col gap-4 pb-1">
            <Field label="Name">
              <Input
                size="sm"
                value={name}
                autoFocus
                onChange={(e) => setName(e.target.value)}
                placeholder="Morning Brief"
                aria-label="Routine name"
              />
            </Field>

            <Field label="Description">
              <Input
                size="sm"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="What this routine produces, in one line"
                aria-label="Routine description"
              />
            </Field>

            <div className="grid gap-4 sm:grid-cols-2">
              <Field label="Agent">
                <Select
                  size="md"
                  value={agent}
                  onChange={setAgent}
                  options={agentOptions}
                  ariaLabel="Agent"
                />
              </Field>
              <Field label="Icon">
                <Select
                  size="md"
                  value={icon}
                  onChange={setIcon}
                  options={iconOptions}
                  ariaLabel="Icon"
                />
              </Field>
            </div>

            <Field label="Schedule">
              <Segmented
                size="xs"
                value={kind}
                onChange={changeKind}
                options={KIND_OPTIONS}
                label="Schedule kind"
              />
            </Field>

            {/* Keyed on the kind so a new control fades in rather than the
                old one's label being rewritten under the cursor. */}
            <div key={kind} className="animate-fade-in">
              <ScheduleControl
                kind={kind}
                expression={expression}
                onChange={setExpression}
                check={check}
              />
            </div>

            <Field label="Playbook">
              <Textarea
                value={markdown}
                onChange={(e) => setMarkdown(e.target.value)}
                spellCheck={false}
                rows={12}
                aria-label="Playbook markdown"
                className="min-h-[16rem] rounded-xl px-3.5 py-3 font-mono text-[12.5px] leading-relaxed"
              />
            </Field>
          </div>
        </ScrollArea>
      </DialogBody>

      <DialogFooter>
        <Button
          variant="secondary"
          size="pill"
          className="w-full sm:w-auto"
          onClick={() => onOpenChange(false)}
        >
          Cancel
        </Button>
        <Button
          variant="primary"
          size="pill"
          className="w-full sm:w-auto"
          disabled={!trimmed || !agent || !check.valid}
          onClick={submit}
        >
          Create routine
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

function Field({ label, children }) {
  // a <label> would forward its click into the Select's trigger and reopen it
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-xs text-foreground/90">{label}</span>
      {children}
    </div>
  );
}
