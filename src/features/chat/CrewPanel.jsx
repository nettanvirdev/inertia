import * as React from "react";
import {
  ChevronRight,
  Hand,
  Pause,
  Play,
  RotateCcw,
  Send,
  Users,
  X,
  Square,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { IconButton } from "@/components/ui/icon-button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Segmented } from "@/components/ui/segmented";
import { Spinner } from "@/components/ui/spinner";
import { Markdown } from "@/features/chat/Markdown";
import { MessageParts } from "@/features/chat/MessageParts";
import { groupParts } from "@/features/chat/tool-groups";
import { useSwap } from "@/features/chat/arrival";

import {
  asTree,
  cancelRun,
  followUpRun,
  interruptRun,
  loadTimeline,
  pauseRun,
  restartRun,
  resumeRun,
  watchRun,
} from "@/lib/crew";
import { isActive, isWorking } from "@shared/crew";
import { formatCost, costOf, priceFor, totalTokens, formatTokens } from "@shared/usage";
import { CrewTimeline } from "@/features/chat/CrewTimeline";

/**
 * What the team is doing.
 *
 * The complaint this answers is that a subagent was a spinner with a name on
 * it. You could see that something had been delegated and you could see the
 * report when it came back, and in between - which for a real piece of work is
 * most of the time - there was nothing: no output, no idea which step it was
 * on, no way to stop one that had clearly gone wrong, and no way to look at
 * what it did once it was over.
 *
 * So the unit here is the run, and every run shows the same five things: what
 * it was asked, what it is doing right now, what it has done so far, what it
 * has actually written, and what it returned. The tree is the fourth: who
 * started whom, drawn by indentation rather than by lines, because the depth is
 * usually two and a diagram of two levels is decoration.
 *
 * ## Three levels of detail, and why the middle one is the default
 *
 * Compact is for glancing: a row per run, its status, its activity. Detailed
 * adds the brief and the live output, which is what someone actually wants when
 * they open this. Debug adds the event log with timestamps, which is what you
 * want twice a month and which makes every other view harder to read.
 */

const STATUS_VARIANT = {
  queued: "neutral",
  running: "info",
  paused: "warning",
  done: "success",
  failed: "danger",
  // Not a warning. A cancelled run is one somebody stopped on purpose, and
  // colouring their own decision amber puts an alarm next to the button they
  // just pressed.
  cancelled: "neutral",
  // Amber, unlike cancelled: an interrupted run is waiting to be told what to
  // do next, and a row that looks finished would never get told.
  interrupted: "warning",
};

/**
 * A new brief for a run that has settled.
 *
 * Inline rather than a dialog, because the thing being answered - what the run
 * did - is directly above it, and a dialog would cover exactly that. Clears
 * on send; the row's status flipping to queued is the confirmation.
 */
function FollowUp({ run }) {
  const [text, setText] = React.useState("");
  const [problem, setProblem] = React.useState(null);
  const send = () => {
    const prompt = text.trim();
    if (!prompt) return;
    setProblem(null);
    followUpRun(run.id, prompt)
      .then((result) => {
        if (result?.started === false) setProblem(result.reason ?? "That did not start.");
        else setText("");
      })
      .catch((error) => setProblem(error?.message ?? "That did not start."));
  };
  return (
    <section>
      <h4 className="text-muted-foreground text-[10px] font-medium tracking-[0.08em] uppercase">
        Follow up
      </h4>
      <div className="mt-1 flex items-center gap-1.5">
        <input
          value={text}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              send();
            }
          }}
          placeholder="What should it do next? It remembers what it did."
          aria-label={`Follow up with ${run.agentName}`}
          className="bg-input focus-visible:bg-input-focus min-w-0 flex-1 rounded-md px-2 py-1 text-xs outline-none"
        />
        <IconButton size="sm" label="Send the follow-up" onClick={send} disabled={!text.trim()}>
          <Send />
        </IconButton>
      </div>
      <Collapse open={Boolean(problem)}>
        <p className="text-destructive-ink mt-1 text-[11px]">{problem}</p>
      </Collapse>
    </section>
  );
}

function elapsed(run) {
  const end = run.endedAt ?? Date.now();
  const seconds = Math.max(0, Math.round((end - run.startedAt) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${seconds % 60}s`;
}

/**
 * What this run has cost, when the model it ran on has a price.
 *
 * Shown per run rather than only in total, because the decision the number
 * informs is per run: whether spawning that fifth helper was worth it is not a
 * question a single figure at the bottom can answer.
 */
function Cost({ usage, model, prices }) {
  if (!usage) return null;
  const tokens = totalTokens(usage);
  if (!tokens) return null;
  const rate = model ? priceFor(model, prices) : null;
  const cost = rate ? costOf(usage, rate) : null;
  return (
    <span className="text-muted-foreground tabular-nums">
      {formatTokens(tokens)}
      {cost ? ` · ${formatCost(cost)}` : ""}
    </span>
  );
}

/**
 * What the helper has done so far, or what it did.
 *
 * The parts come from the backend already folded into the shape the window
 * folds a reply into, so this is the transcript's own renderer over the
 * transcript's own shape - not a second rendering of the same events.
 *
 * `message` is only sent for the run this column has open (see `watch` in
 * `bridge/crew.js`), so a run that has just been opened draws its summary
 * until the next snapshot arrives a moment later rather than flashing empty.
 */
function Transcript({ run }) {
  const parts = run.message?.parts;
  const items = React.useMemo(() => groupParts(parts ?? []), [parts]);
  const live = isActive(run.status);

  if (!parts?.length) {
    const body = (run.result ?? run.text ?? "").trim();
    if (!body) {
      return (
        <p className="text-muted-foreground py-1 text-xs">
          {live ? "Working..." : "Nothing was written down for this run."}
        </p>
      );
    }
    return <Markdown>{body}</Markdown>;
  }

  return <MessageParts items={items} streaming={live} fallback={run.text} />;
}

function Run({ run, prices, open, onToggle, onRestart }) {
  const live = isActive(run.status);

  return (
    <li
      className="card-surface-subtle rounded-xl px-2.5 py-2"
      style={{ marginLeft: `${Math.min(run.indent, 4) * 14}px` }}
    >
      <div className="flex items-start gap-2">
        <button
          type="button"
          onClick={onToggle}
          className="text-muted-foreground hover:text-foreground mt-0.5 shrink-0"
          aria-label={open ? "Collapse" : "Expand"}
        >
          {/* One chevron turned rather than two swapped, so the control never
              changes shape - the same rule as every other disclosure. */}
          <ChevronRight
            className={cn(
              "size-3.5 transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
              open && "rotate-90"
            )}
          />
        </button>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="truncate text-[13px] font-medium">{run.agentName}</span>
            <Badge variant={STATUS_VARIANT[run.status] ?? "neutral"} size="sm">
              {run.status}
            </Badge>
            {isWorking(run.status) ? <Spinner className="size-3" /> : null}
          </div>
          <p className="text-muted-foreground truncate text-xs">{run.description}</p>
          <div className="text-muted-foreground mt-0.5 flex flex-wrap items-center gap-x-2 text-[11px]">
            <span className="font-mono">{run.id}</span>
            <span>{live ? run.activity : elapsed(run)}</span>
            {run.steps?.length ? (
              <span>
                {run.steps.length} step{run.steps.length === 1 ? "" : "s"}
              </span>
            ) : null}
            <Cost usage={run.usage} model={run.model} prices={prices} />
          </div>
        </div>

        <div className="flex shrink-0 items-center">
          {/* Pause is only offered while something is actually happening. On a
              run that has already stopped it would be a button that does
              nothing, which is worse than an absent one. */}
          {isWorking(run.status) ? (
            <IconButton
              size="sm"
              label="Pause this run"
              onClick={() => pauseRun(run.id).catch(() => {})}
            >
              <Pause />
            </IconButton>
          ) : null}
          {/* Interrupt is the third way to stop something, and it earns its
              button: pause keeps the turn to resume, stop throws the run away,
              this ends the turn and keeps the run for a new brief. */}
          {isWorking(run.status) ? (
            <IconButton
              size="sm"
              label="Interrupt this run, keeping it for a follow-up"
              onClick={() => interruptRun(run.id).catch(() => {})}
            >
              <Hand />
            </IconButton>
          ) : null}
          {run.status === "paused" ? (
            <IconButton
              size="sm"
              label="Resume this run"
              onClick={() => resumeRun(run.id).catch(() => {})}
            >
              <Play />
            </IconButton>
          ) : null}
          {/* Restart is for after it went wrong, which is the only time anyone
              wants it - but it is offered on a finished run too, because "do
              that again" is a real thing to want. */}
          {!isWorking(run.status) ? (
            <IconButton size="sm" label="Run this brief again" onClick={() => onRestart?.(run)}>
              <RotateCcw />
            </IconButton>
          ) : null}
          {live ? (
            <IconButton
              size="sm"
              label="Stop this run"
              onClick={() => cancelRun(run.id).catch(() => {})}
            >
              <Square className="fill-current" />
            </IconButton>
          ) : null}
        </div>
      </div>

      <Collapse open={open} innerClassName="mt-2 space-y-2.5 pl-5">
        {/* A helper's run, read the way a reply is read.
         *
         * `chat-scale` shrinks the type and everything built on it: this is
         * a column beside the conversation, not the conversation, and a tool
         * card at full size in a 24rem column wraps its own title.
         *
         * The brief takes the place of the user's message because that is
         * exactly what it is - the thing this agent was asked to do - and
         * putting it anywhere else would make a run the only transcript in
         * the app that starts with nobody having said anything. */}
        <div className="chat-scale">
          <div className="mb-1 flex justify-end">
            <div className="max-w-[85%] rounded-2xl rounded-br-md bg-user-message-background px-3 py-2 wrap-break-word">
              <Markdown>{run.prompt || run.description || ""}</Markdown>
            </div>
          </div>
          <Transcript run={run} />
        </div>
        {run.error ? <p className="text-xs text-destructive-ink">{run.error}</p> : null}

        {run.restartOf || run.restartedAs || run.followUps ? (
          <p className="text-muted-foreground text-[11px]">
            {run.restartOf ? `Restart of ${run.restartOf}.` : null}
            {run.restartedAs ? ` Restarted as ${run.restartedAs}.` : null}
            {run.followUps
              ? ` Followed up ${run.followUps} time${run.followUps === 1 ? "" : "s"}.`
              : null}
          </p>
        ) : null}

        {run.canFollowUp ? <FollowUp run={run} /> : null}

        {run.inbox?.length ? (
          <section>
            <h4 className="text-muted-foreground text-[10px] font-medium tracking-[0.08em] uppercase">
              Messages
            </h4>
            <ul className="text-foreground-secondary mt-0.5 space-y-0.5 text-xs">
              {run.inbox.slice(-6).map((message, index) => (
                <li key={`${message.at}-${index}`}>
                  <span className="font-medium">{message.fromName}:</span> {message.text}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
      </Collapse>
    </li>
  );
}

/**
 * The panel itself.
 *
 * A sheet rather than a third docked column: the chat already has a right
 * panel, and two of them at once leaves the conversation - the thing everything
 * else is about - as a strip down the middle. It is opened deliberately, from
 * an icon that only exists while there is something to look at.
 */
/**
 * What the whole team has cost so far.
 *
 * One number, in the header, because with no ceiling on how many runs an agent
 * may start this is the only limit that actually exists - and a limit nobody
 * can see is not a limit. Per-run figures answer "was that helper worth it";
 * this answers "should I stop all of them".
 */
function Total({ runs, prices }) {
  const { tokens, cost } = React.useMemo(() => {
    let tokenSum = 0;
    let costSum = 0;
    for (const run of runs ?? []) {
      if (!run.usage) continue;
      tokenSum += totalTokens(run.usage) ?? 0;
      const rate = run.model ? priceFor(run.model, prices) : null;
      if (rate) costSum += costOf(run.usage, rate) ?? 0;
    }
    return { tokens: tokenSum, cost: costSum };
  }, [runs, prices]);

  if (!tokens) return null;
  return (
    <span className="text-muted-foreground tabular-nums">
      {formatTokens(tokens)}
      {cost ? ` · ${formatCost(cost)}` : ""}
    </span>
  );
}

export function CrewPanel({ open, onOpenChange, conversationId, runs, prices }) {
  const [tab, setTab] = React.useState("runs");
  /**
   * One run open at a time.
   *
   * A run is a whole transcript now, not three lines of summary, and two of
   * those in a 24rem column is a column nobody can find anything in. It is
   * also what makes the pushed transcript affordable: main only sends the
   * parts for the run being read, so "one open" is the thing that keeps a
   * dozen live helpers from costing a dozen transcripts per token.
   */
  const [openId, setOpenId] = React.useState(null);
  const [events, setEvents] = React.useState([]);

  const tree = React.useMemo(() => asTree(runs), [runs]);
  const active = tree.filter((run) => isActive(run.status));
  const working = tree.filter((run) => isWorking(run.status));

  /**
   * The full event log, fetched only while the timeline is on screen.
   *
   * Re-fetched on every change to the runs, which is what keeps a live chart
   * live. It is a main-process map lookup and a sort, not a query, so the cost
   * of being current here is far below the cost of pushing the same history
   * with every snapshot for the benefit of a tab nobody has opened.
   */
  React.useEffect(() => {
    if (!open || tab !== "timeline" || !conversationId) return;
    let alive = true;
    loadTimeline(conversationId)
      .then((rows) => {
        if (alive) setEvents(rows);
      })
      .catch(() => {
        if (alive) setEvents([]);
      });
    return () => {
      alive = false;
    };
  }, [open, tab, conversationId, runs]);

  // A run that is still going opens itself, once. Someone who opens this panel
  // while three helpers are working wants to see them working, and clicking
  // three disclosure triangles to find that out is the version of this that
  // makes people stop opening it.
  const seen = React.useRef(new Set());
  React.useEffect(() => {
    if (!open) return;
    const started = working.find((run) => !seen.current.has(run.id));
    if (!started) return;
    seen.current.add(started.id);
    setOpenId(started.id);
  }, [open, working]);

  /**
   * Tell main which run is being read, and stop when nothing is.
   *
   * Also on close: a column nobody is looking at should not be sent a
   * transcript, and the cleanup is what makes that true when the panel is
   * closed rather than merely switched.
   */
  React.useEffect(() => {
    if (!conversationId) return undefined;
    watchRun(conversationId, open ? openId : null);
    return () => watchRun(conversationId, null);
  }, [conversationId, openId, open]);

  const toggle = (id) => setOpenId((current) => (current === id ? null : id));

  const stopEverything = () => {
    for (const run of active) cancelRun(run.id, { descendants: true }).catch(() => {});
  };

  const pauseEverything = () => {
    for (const run of working) pauseRun(run.id, { descendants: true }).catch(() => {});
  };

  const resumeEverything = () => {
    for (const run of tree) if (run.status === "paused") resumeRun(run.id).catch(() => {});
  };

  const restart = (run) => {
    restartRun(run.id).catch(() => {});
  };

  const paused = tree.filter((run) => run.status === "paused");

  // Runs and the timeline are two views of one team, so one gives way to the
  // other with a fade rather than a cut.
  const swap = useSwap(tab);

  // The dock owns the column, its width and its share of the height; this owns
  // what is in it. It is not rendered at all when closed, so a shut panel costs
  // no transcript, no timeline fetch and no subscription.
  return (
    <div className="flex h-full min-h-0 w-full flex-col">
      <header className="border-border flex items-center gap-2 border-b px-4 py-3">
        <Users className="size-4 shrink-0" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">Agents</h2>
          <p className="text-muted-foreground flex items-center gap-2 text-xs">
            <span>
              {working.length
                ? `${working.length} running of ${tree.length}`
                : paused.length
                  ? `${paused.length} paused of ${tree.length}`
                  : `${tree.length} in this conversation`}
            </span>
            <Total runs={runs} prices={prices} />
          </p>
        </div>
        <IconButton size="sm" label="Close" onClick={() => onOpenChange?.(false)}>
          <X />
        </IconButton>
      </header>

      <div className="border-border flex items-center gap-2 border-b px-4 py-2">
        <Segmented
          size="sm"
          label="What to show"
          value={tab}
          onChange={setTab}
          options={[
            { value: "runs", label: "Runs" },
            { value: "timeline", label: "Timeline" },
          ]}
        />
        <div className="ml-auto flex items-center gap-1">
          {working.length ? (
            <Button size="sm" variant="ghost" onClick={pauseEverything}>
              Pause all
            </Button>
          ) : null}
          {paused.length ? (
            <Button size="sm" variant="ghost" onClick={resumeEverything}>
              Resume all
            </Button>
          ) : null}
          {active.length ? (
            <Button size="sm" variant="ghost" onClick={stopEverything}>
              Stop all
            </Button>
          ) : null}
        </div>
      </div>

      {tab === "timeline" ? (
        <div
          key="timeline"
          onAnimationEnd={swap.onAnimationEnd}
          className={cn("flex min-h-0 flex-1 flex-col", swap.swapping && "animate-fade-in")}
        >
          <CrewTimeline runs={tree} events={events} />
        </div>
      ) : (
        // `min-h-0`, or a flex child refuses to shrink below its content and
        // the column overflows instead of scrolling.
        <ScrollArea
          key="runs"
          onAnimationEnd={swap.onAnimationEnd}
          className={cn("min-h-0 flex-1", swap.swapping && "animate-fade-in")}
        >
          {tree.length ? (
            <ul className="space-y-1.5 px-3 py-3">
              {tree.map((run) => (
                <Run
                  key={run.id}
                  run={run}
                  prices={prices}
                  open={openId === run.id}
                  onToggle={() => toggle(run.id)}
                  onRestart={restart}
                />
              ))}
            </ul>
          ) : (
            <p className="text-muted-foreground px-4 py-8 text-center text-xs">
              Nothing has been delegated in this conversation yet.
            </p>
          )}
        </ScrollArea>
      )}
    </div>
  );
}
