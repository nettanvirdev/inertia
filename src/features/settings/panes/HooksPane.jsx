import * as React from "react";
import { ExternalLink, Workflow } from "@/components/icons";
import { cn } from "@/lib/utils";
import { relativeTime } from "@/data";
import { hooks as hooksBridge, isHooksAvailable } from "@/lib/hooks";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { SettingsCard, SettingsRow, SettingsSection } from "../SettingsRow";

/**
 * What the hooks are doing.
 *
 * A hook is a program, and the place to write a program is an editor, so this
 * pane does not try to be one. It answers the three questions a person has
 * about hooks they wrote elsewhere: are they loaded, is anything wrong with
 * them, and what did they do. The two controls are the master switch and a
 * way to get the file open - or written, for someone starting from nothing.
 */

const STATUS_TONE = {
  ok: "success",
  blocked: "warning",
  stopped: "warning",
  failed: "danger",
  timeout: "danger",
  cancelled: "neutral",
};

const STATUS_LABEL = {
  ok: "Passed",
  blocked: "Blocked",
  stopped: "Ended the turn",
  failed: "Failed",
  timeout: "Timed out",
  cancelled: "Cancelled",
};

/**
 * How many runs the list is tall before it starts scrolling.
 *
 * The buffer holds a hundred, and a hundred rows below everything else turned
 * the pane into a log file with a settings page on top of it. Five is enough
 * to see that a hook fired and what it did last, and the rest are a scroll
 * away rather than gone.
 */
const VISIBLE_RUNS = 5;

/** The events in the order they happen, with one line each. */
const EVENT_HELP = {
  SessionStart: "The first turn of a conversation",
  UserPromptSubmit: "A message, before it is sent",
  PreToolUse: "Before a tool runs",
  PermissionRequest: "Before you are asked to approve a call",
  PostToolUse: "After a tool ran",
  Notification: "A permission card is about to show",
  PreCompact: "The transcript is about to be summarised",
  PostCompact: "After it was",
  SubagentStart: "A spawned agent begins",
  SubagentStop: "A spawned agent wants to stop",
  Stop: "The agent wants to stop",
  StopFailure: "The turn ended in a provider error",
};

const SOURCE_LABEL = { workspace: "Workspace", project: "Project", agent: "Agent" };

function sourceOf(handler) {
  const key = String(handler.source ?? "").split(":")[0];
  return SOURCE_LABEL[key] ?? handler.source;
}

function HandlerRow({ handler }) {
  const what = handler.type === "prompt" ? handler.prompt : handler.args ? `${handler.command} ${handler.args.join(" ")}` : handler.command;
  return (
    <SettingsRow
      label={
        <span className="flex min-w-0 items-center gap-2">
          <span className={cn("truncate", !handler.enabled && "text-muted-foreground line-through")}>
            {handler.name ?? (handler.type === "prompt" ? "Prompt" : "Command")}
          </span>
          {handler.matcher ? (
            <Badge variant="neutral" size="sm" className="shrink-0 font-mono">
              {handler.matcher}
            </Badge>
          ) : null}
        </span>
      }
      description={
        <span className="block truncate font-mono text-[11px]" title={what ?? ""}>
          {what}
        </span>
      }
      control={
        <span className="flex items-center gap-1.5">
          <Badge variant="neutral" size="sm">
            {handler.type === "prompt" ? "prompt" : "command"}
          </Badge>
          <Badge variant="neutral" size="sm">
            {sourceOf(handler)}
          </Badge>
          {!handler.enabled ? (
            <Badge variant="warning" size="sm">
              off
            </Badge>
          ) : null}
        </span>
      }
    />
  );
}

function RunRow({ run }) {
  const label = STATUS_LABEL[run.status] ?? run.status;
  return (
    <SettingsRow
      label={
        <span className="flex min-w-0 items-center gap-2">
          <span className="shrink-0 text-muted-foreground">{run.event}</span>
          <span className="truncate">{run.handler}</span>
          {run.subject ? (
            <Badge variant="neutral" size="sm" className="shrink-0 font-mono">
              {run.subject}
            </Badge>
          ) : null}
        </span>
      }
      description={run.reason ?? run.error ?? (run.stderr ? run.stderr.split("\n")[0] : null)}
      control={
        <span className="flex items-center gap-2">
          <span className="text-[11px] tabular-nums text-muted-foreground">
            {run.durationMs != null ? `${run.durationMs} ms` : ""}
          </span>
          <Badge variant={STATUS_TONE[run.status] ?? "neutral"} size="sm" dot>
            {label}
          </Badge>
          <span className="text-[11px] text-muted-foreground">{relativeTime(new Date(run.at).toISOString())}</span>
        </span>
      }
    />
  );
}

export function HooksPane() {
  const { toast } = useToast();
  const available = isHooksAvailable();
  const [loaded, setLoaded] = React.useState(null);
  const [runs, setRuns] = React.useState([]);
  const [busy, setBusy] = React.useState(false);
  const runList = React.useRef(null);
  const [runsHeight, setRunsHeight] = React.useState(null);

  /**
   * Cap the list at five rows, measured rather than assumed.
   *
   * A row is a different height depending on whether the run had a reason to
   * report, so any fixed pixel figure is right for one kind of list and wrong
   * for the other - it would cut the fifth row in half exactly when something
   * has gone wrong and the row got taller. Measuring where the sixth row
   * starts gives the true height of the first five, whatever they contain.
   */
  React.useLayoutEffect(() => {
    const node = runList.current;
    if (!node) return;
    const rows = node.children;
    if (rows.length <= VISIBLE_RUNS) {
      setRunsHeight(null);
      return;
    }
    setRunsHeight(rows[VISIBLE_RUNS].offsetTop - rows[0].offsetTop);
  }, [runs]);

  const reload = React.useCallback(async () => {
    if (!available) return;
    try {
      const [list, recent] = await Promise.all([hooksBridge.list({}), hooksBridge.recent()]);
      setLoaded(list);
      setRuns(Array.isArray(recent) ? recent : []);
    } catch (error) {
      toast({ variant: "danger", title: "Could not read hooks", description: error?.message ?? String(error) });
    }
  }, [available, toast]);

  React.useEffect(() => {
    reload();
  }, [reload]);

  // Runs arrive as they happen, so the list moves while an agent works.
  React.useEffect(() => {
    if (!available) return undefined;
    return hooksBridge.onRun((run) => setRuns((prev) => [run, ...prev].slice(0, 100)));
  }, [available]);

  const workspace = loaded?.sources?.find((s) => s.source === "workspace") ?? null;
  const enabled = workspace ? workspace.enabled !== false : true;
  const handlers = React.useMemo(() => loaded?.handlers ?? [], [loaded]);
  const byEvent = React.useMemo(() => {
    const map = new Map();
    for (const handler of handlers) {
      if (!map.has(handler.event)) map.set(handler.event, []);
      map.get(handler.event).push(handler);
    }
    return [...map.entries()].sort(
      (a, b) => Object.keys(EVENT_HELP).indexOf(a[0]) - Object.keys(EVENT_HELP).indexOf(b[0])
    );
  }, [handlers]);

  const toggle = async (next) => {
    setBusy(true);
    try {
      await hooksBridge.setEnabled(next);
      await reload();
    } catch (error) {
      toast({ variant: "danger", title: "Could not change that", description: error?.message ?? String(error) });
    } finally {
      setBusy(false);
    }
  };

  const writeExample = async () => {
    setBusy(true);
    try {
      const result = await hooksBridge.writeExample();
      toast({ title: "Example written", description: result?.path ?? "hooks/hooks.json" });
      await reload();
    } catch (error) {
      toast({ variant: "danger", title: "Could not write the example", description: error?.message ?? String(error) });
    } finally {
      setBusy(false);
    }
  };

  const reveal = () => hooksBridge.reveal().catch((error) => toast({ variant: "danger", title: "Could not open it", description: error?.message ?? String(error) }));

  if (!available) {
    return (
      <div className="w-full">
        <SettingsSection flat
          title="Hooks"
          description="Programs of your own that run before and after an agent acts, and when it wants to stop."
        >
          <SettingsCard>
            <p className="text-xs text-muted-foreground">
              Hooks run in the desktop app. This preview has no agent loop behind it.
            </p>
          </SettingsCard>
        </SettingsSection>
      </div>
    );
  }

  return (
    <div className="w-full">
      <SettingsSection
        title="Hooks"
        description="Programs of your own that run at named moments of an agent's work: before a tool runs, after it, when the agent wants to stop. Edit hooks/hooks.json in the workspace; it is read fresh for every turn."
      >
        <SettingsRow
          label="Run hooks"
          description={
            workspace?.present
              ? `${handlers.filter((h) => h.enabled).length} loaded from ${loaded?.sources?.filter((s) => s.present).length ?? 0} place${(loaded?.sources?.filter((s) => s.present).length ?? 0) === 1 ? "" : "s"}.`
              : "No hooks file yet. Write an example to start from, or create hooks/hooks.json yourself."
          }
          htmlFor="hooks-enabled"
          control={
            <Switch id="hooks-enabled" checked={enabled} disabled={busy || !workspace?.present} onCheckedChange={toggle} />
          }
        />
        <SettingsRow
          label="The file"
          description={loaded?.path ?? "hooks/hooks.json"}
          control={
            <span className="flex items-center gap-1.5">
              {!workspace?.present ? (
                <Button variant="secondary" size="xs" disabled={busy} onClick={writeExample}>
                  Write an example
                </Button>
              ) : null}
              <Button variant="ghost" size="xs" onClick={reveal}>
                <ExternalLink />
                Open
              </Button>
            </span>
          }
        />
      </SettingsSection>

      {loaded?.warnings?.length ? (
        <SettingsSection flat title="Problems" description="Fix these and the next turn picks the change up.">
          <SettingsCard className="flex flex-col gap-1">
            {loaded.warnings.map((warning, index) => (
              <p key={index} className="text-xs text-warning-ink">
                {warning}
              </p>
            ))}
          </SettingsCard>
        </SettingsSection>
      ) : null}

      <SettingsSection
        title="Loaded"
        description="Every handler in force, by the event it answers to. A project's own .inertia/hooks.json and an agent's hooks join these when a turn runs."
      >
        {byEvent.length ? (
          byEvent.map(([event, list]) => (
            <div key={event}>
              <div className="flex items-baseline gap-2 px-4 pt-3 pb-1">
                <span className="text-[0.8125rem] font-medium text-foreground">{event}</span>
                <span className="text-[11px] text-muted-foreground">{EVENT_HELP[event]}</span>
              </div>
              {list.map((handler) => (
                <HandlerRow key={handler.id} handler={handler} />
              ))}
            </div>
          ))
        ) : (
          <div className="flex items-start gap-3 px-4 py-4">
            <Workflow className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <div className="text-xs leading-relaxed text-muted-foreground">
              <p>Nothing is loaded. A hooks file lists events, and under each event the programs to run:</p>
              <pre className="mt-2 overflow-x-auto rounded-lg fill-whisper p-3 font-mono text-[11px] leading-relaxed text-foreground">
{`{
  "PreToolUse": [
    { "matcher": "shell",
      "hooks": [{ "type": "command", "command": "node hooks/check.js" }] }
  ],
  "Stop": [
    { "hooks": [{ "type": "prompt",
                  "prompt": "Block if the tests were never run." }] }
  ]
}`}
              </pre>
              <p className="mt-2">
                A command gets the event as JSON on stdin; exit 2 refuses it. A prompt asks the model.
                The full contract is in docs/hooks.md.
              </p>
            </div>
          </div>
        )}
      </SettingsSection>

      <SettingsSection
        title="Recent runs"
        description="What your hooks did, newest first. Hooks that simply passed are here too."
        contentRef={runList}
        contentClassName={runsHeight == null ? undefined : "overflow-y-auto no-scrollbar"}
        contentStyle={runsHeight == null ? undefined : { maxHeight: runsHeight }}
      >
        {runs.length ? (
          runs.slice(0, 30).map((run, index) => <RunRow key={`${run.at}-${index}`} run={run} />)
        ) : (
          <div className="px-4 py-4 text-xs text-muted-foreground">Nothing has run since the app started.</div>
        )}
      </SettingsSection>
    </div>
  );
}
