import * as React from "react";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Slider } from "@/components/ui/slider";
import { isActive } from "@shared/crew";

/**
 * What ran, when, and next to what.
 *
 * The claim the concurrency work makes is that four agents worked at the same
 * time. A list of runs cannot show that - a list looks identical whether they
 * overlapped or queued up one behind the other, which is the entire difference
 * between the new shape and the old one. Bars on a shared clock show it in a
 * glance, and show its absence just as plainly: a staircase of bars is a model
 * that spawned and collected one at a time, and that is worth catching.
 *
 * Underneath, the same events merged into one stream. Bars answer "did this
 * overlap"; the stream answers "what actually happened, in order", which is the
 * question you have after something went wrong.
 *
 * ## Replaying, honestly
 *
 * The scrubber moves a line across the chart and cuts the stream at that
 * moment, so you can watch the shape of the work build up. It is replaying a
 * recorded event log, not re-running anything: what you see is what was written
 * down while it happened, and events beyond the per-run cap were dropped when
 * they aged out. Calling it a replay of the work itself would be a promise the
 * data cannot keep.
 */

const BAR = {
  queued: "bg-muted-foreground/25",
  running: "bg-info",
  paused: "bg-warning",
  done: "bg-success",
  failed: "bg-destructive",
  cancelled: "bg-muted-foreground/40",
};

const KIND_VARIANT = {
  spawned: "info",
  started: "info",
  tool: "neutral",
  message: "info",
  paused: "warning",
  resumed: "info",
  restarted: "warning",
  result: "success",
  failed: "danger",
  cancelled: "neutral",
};

function clock(at) {
  return new Date(at).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/**
 * The window the chart covers.
 *
 * `now` is the right end while anything is still going, so a live run's bar
 * grows rather than stopping at whatever its last recorded event was. Once
 * everything has settled the span is fixed, and the chart stops moving - which
 * is what makes it readable after the fact.
 */
function spanOf(runs, now) {
  if (!runs.length) return { from: now, to: now + 1 };
  const from = Math.min(...runs.map((run) => run.startedAt));
  const live = runs.some((run) => isActive(run.status));
  const to = live ? now : Math.max(...runs.map((run) => run.endedAt ?? run.startedAt));
  // A team that started and finished inside the same millisecond would divide
  // by zero. One second of floor costs nothing and is never visible.
  return { from, to: Math.max(to, from + 1000) };
}

export function CrewTimeline({ runs, events }) {
  const [now, setNow] = React.useState(() => Date.now());
  const [at, setAt] = React.useState(null); // null = live, following the end

  const live = runs.some((run) => isActive(run.status));

  // Only while something is running. A finished chart that keeps re-rendering
  // once a second is a background task nobody asked for.
  React.useEffect(() => {
    if (!live) return undefined;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [live]);

  const span = React.useMemo(() => spanOf(runs, now), [runs, now]);
  const width = span.to - span.from;
  const cut = at ?? span.to;

  const shown = React.useMemo(
    () => (events ?? []).filter((event) => event.at <= cut).slice(-200),
    [events, cut]
  );

  const place = (run) => {
    const start = Math.max(0, run.startedAt - span.from);
    const end = Math.min(width, (run.endedAt ?? span.to) - span.from);
    return {
      left: `${(start / width) * 100}%`,
      // A run that lasted a moment still gets something you can see and hover.
      width: `${Math.max(1.5, ((end - start) / width) * 100)}%`,
    };
  };

  if (!runs.length) {
    return (
      <p className="text-muted-foreground px-4 py-8 text-center text-xs">
        Nothing has run in this conversation yet.
      </p>
    );
  }

  // Where the scrubber is, as a fraction of the chart. Drawn over the bars
  // rather than by clipping them: a bar that stopped at the playhead would say
  // the run ended there, which is the one thing it must not say.
  const playhead = `${Math.min(100, Math.max(0, ((cut - span.from) / width) * 100))}%`;
  const scrubbing = at !== null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="border-border space-y-1.5 border-b px-4 py-3">
        {/* The bars and the playhead share one positioning context, so the line
            spans every row instead of being drawn once per bar. `pl-[5.5rem]`
            is the label column plus its gap. */}
        <div className="relative">
          <div className="space-y-1.5">
            {runs.map((run) => {
              const style = place(run);
              return (
                <div key={run.id} className="flex items-center gap-2">
                  <span className="text-muted-foreground w-20 shrink-0 truncate text-[11px]">
                    {run.agentName}
                  </span>
                  <span className="bg-secondary relative h-2.5 flex-1 overflow-hidden rounded-full">
                    <span
                      className={`absolute inset-y-0 rounded-full ${BAR[run.status] ?? BAR.queued}`}
                      style={style}
                      title={`${run.description} - ${run.status}`}
                    />
                  </span>
                </div>
              );
            })}
          </div>
          {scrubbing ? (
            <div className="pointer-events-none absolute inset-y-0 right-0 left-[5.5rem]">
              <div
                className="bg-foreground/60 absolute inset-y-0 w-px"
                style={{ left: playhead }}
              />
            </div>
          ) : null}
        </div>

        <div className="flex items-center gap-2 pt-1">
          <span className="text-muted-foreground w-20 shrink-0 text-[11px] tabular-nums">
            {scrubbing ? clock(cut) : "Live"}
          </span>
          {/* The app's own slider rather than <input type="range">. A native
              range takes the platform's styling, which in a dark panel is a
              thick white bar - the loudest thing on the screen, sitting under
              a chart it is meant to serve. */}
          <Slider
            className="flex-1"
            tone="muted"
            label="Move through the run"
            min={span.from}
            max={span.to}
            step={100}
            value={cut}
            formatValue={(value) => clock(value)}
            onChange={(value) => {
              // Dragging to the far right means "follow along" rather than
              // "pin me to this millisecond", or the view would freeze the
              // moment you let go of a live chart.
              setAt(value >= span.to - 200 ? null : value);
            }}
          />
        </div>
      </div>

      <ScrollArea className="flex-1">
        <ul className="space-y-0.5 px-4 py-2">
          {shown.map((event, index) => (
            <li key={`${event.at}-${event.runId}-${index}`} className="flex items-start gap-2 text-[11px]">
              <span className="text-muted-foreground w-[4.5rem] shrink-0 whitespace-nowrap font-mono">
                {clock(event.at)}
              </span>
              <Badge variant={KIND_VARIANT[event.kind] ?? "neutral"} size="sm">
                {event.kind}
              </Badge>
              <span className="text-muted-foreground shrink-0">{event.agentName}</span>
              <span className="text-foreground/80 min-w-0 flex-1 break-words">{event.text}</span>
            </li>
          ))}
          {!shown.length ? (
            <li className="text-muted-foreground py-6 text-center text-xs">
              Nothing had happened yet at this point.
            </li>
          ) : null}
        </ul>
      </ScrollArea>
    </div>
  );
}
