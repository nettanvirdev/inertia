import * as React from "react";
import {
  Brain,
  ChevronDown,
  CalendarClock,
  Eye,
  Hand,
  Icon,
  Maximize2,
  Monitor,
  Pause,
  Plus,
  Settings2,
  Sparkles,
  X,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { computers as machines } from "@/lib/computers";
import { COMPUTER_STATUS_META, PROVIDER_META } from "@/data/computers";
import { MODELS, relativeTime } from "@/data";
import { parseModelRef } from "@shared/providers";
import {
  PRICES_AS_OF,
  addUsage,
  cacheHitRate,
  costOf,
  formatCost,
  formatTokens,
  priceFor,
  totalTokens,
} from "@shared/usage";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { IconButton } from "@/components/ui/icon-button";
import { Meter } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { absoluteTime } from "@/features/routines/run-status";
import { ArtifactDialog } from "@/features/library/ArtifactDialog";
import { ComputerDialog } from "@/features/computers/ComputerDialog";
import {
  FILE_KIND_ICON,
  collectThreadFiles,
  formatBytes,
} from "@/features/chat/thread-files";

/**
 * The chat's right panel: what the agent is working WITH, next to what it is
 * saying. Computer, files, routines, session - the things you would otherwise
 * have to leave the conversation to look up.
 *
 * It exists only beside a live thread and it is togglable, from the computer
 * chip in the composer or the Monitor button in the header. Whether it is open
 * is a persisted preference; there is nothing per-thread to remember, because
 * every section reads from the thread it is currently mounted next to.
 *
 * Layout: a docked column at lg and up, an overlay pinned to the right edge
 * below that - the same element either way, so opening it never remounts and
 * never loses scroll position. While it overlays, Escape closes it: it is
 * covering the conversation at that size, so it has to be as cheap to dismiss
 * as anything else that covers something. Surface is `sidebar`, the same rung the rail on
 * the far side sits on: the chat is then a `background` trough between two
 * equal walls, which is a step the eye reads in light AND dark (card-darker
 * would have been within a hair of the background in dark).
 *
 * Everything below takes its data from the store and nothing calls out to a
 * fixture directly, so wiring this to a backend is a matter of changing where
 * the store gets `computers`, `routines` and `messages` - not of touching this
 * file.
 */

/* ── a collapsible section ─────────────────────────────────────────────────
 * The heading is the button. A chevron rotates rather than swapping glyphs so
 * the control never changes size, and the body folds rather than appearing -
 * and unmounts once folded, so a long routine list costs nothing while it is
 * put away.
 * ────────────────────────────────────────────────────────────────────────── */
function Section({ id, title, count, action, children, defaultOpen = true }) {
  const { chatPanelSections, toggleChatPanelSection } = useApp();
  const flipped = Boolean(chatPanelSections?.[id]);
  const open = defaultOpen ? !flipped : flipped;

  return (
    <section className="flex flex-col">
      <div className="flex h-8 items-center gap-1 pr-1">
        <button
          type="button"
          onClick={() => toggleChatPanelSection(id)}
          aria-expanded={open}
          className={cn(
            "group -ml-1.5 flex min-w-0 flex-1 items-center gap-1 rounded-lg px-1.5 py-1 text-left outline-none",
            "transition-colors duration-75 ease-out hover:fill-nav",
            "focus-visible:fill-nav",
          )}
        >
          <ChevronDown
            aria-hidden="true"
            className={cn(
              "size-3.5 shrink-0 text-muted-foreground transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
              !open && "-rotate-90",
            )}
          />
          <span className="truncate text-[11px] tracking-wide text-muted-foreground">
            {title}
          </span>
          {count != null ? (
            <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/70">
              {count}
            </span>
          ) : null}
        </button>
        {action}
      </div>

      <Collapse open={open} innerClassName="pb-1">
        {children}
      </Collapse>
    </section>
  );
}

/** Label above, value below - the panel's whole vocabulary for a fact. */
function Fact({ label, value, title }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className="truncate text-[10.5px] text-muted-foreground">
        {label}
      </span>
      <span
        className="truncate text-[12px] text-foreground"
        title={title ?? value}
      >
        {value}
      </span>
    </div>
  );
}

function Empty({ children }) {
  return (
    <p className="rounded-xl fill-whisper px-3 py-3 text-[11.5px] leading-relaxed text-muted-foreground">
      {children}
    </p>
  );
}

/* ── computer ─────────────────────────────────────────────────────────────── */

function ComputerSection({ computer, agent, thread }) {
  const { setView, setActiveComputerId } = useApp();
  const [frame, setFrame] = React.useState(null);
  // "view" or "control" while the screen dialog is open, null otherwise.
  const [screen, setScreen] = React.useState(null);

  const running = computer?.status === "running";
  const computerId = computer?.id ?? null;

  // One frame when the section is looked at, and one a minute after that. The
  // computers screen is where someone watches a machine; this is a glance, and
  // polling it at that rate beside a conversation would be a bill on Daytona
  // and a wasted exec on Docker.
  React.useEffect(() => {
    if (!computerId || !running) {
      setFrame(null);
      return undefined;
    }
    let alive = true;
    const take = () =>
      machines
        .screenshot(computerId)
        .then((shot) => {
          if (alive) setFrame(shot);
        })
        .catch(() => {});
    take();
    const timer = window.setInterval(take, 60_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [computerId, running]);

  if (!computer) {
    return (
      <div className="flex flex-col gap-2">
        <Empty>
          No computer is attached to this thread. Without one the agent can
          talk, but it cannot run a command, open a file or drive a browser.
        </Empty>
        <Button
          variant="subtle"
          size="xs"
          className="w-full"
          onClick={() => setView("computers")}
        >
          <Monitor />
          Attach a computer
        </Button>
      </div>
    );
  }

  const status =
    COMPUTER_STATUS_META[computer.status] ?? COMPUTER_STATUS_META.stopped;
  const provider = PROVIDER_META[computer.provider];
  const open = () => {
    setActiveComputerId(computer.id);
    setView("computers");
  };

  return (
    <div className="flex flex-col gap-3">
      {/* A real frame off the machine when it has a display, and a plain
          statement of what it is when it does not. The caption used to be the
          record's `screenshotAlt` - a sentence describing an imaginary desktop,
          written by hand into the fixture. */}
      <div className="relative aspect-[16/10] w-full overflow-hidden rounded-2xl fill-whisper">
        {frame ? (
          // The first frame always replaces the placeholder - there is never
          // one at mount - so it fades over it rather than snapping in.
          <img
            src={frame}
            alt={`The screen of ${computer.name}`}
            className="size-full object-cover animate-fade-in"
          />
        ) : (
          <div className="flex size-full flex-col items-center justify-center gap-1.5 px-5 text-center">
            <Monitor className="size-5 text-muted-foreground/60" aria-hidden="true" />
            <p className="text-[11px] text-muted-foreground">{computer.os || computer.provider}</p>
            <p className="line-clamp-3 text-[10.5px] leading-relaxed text-muted-foreground/70">
              {running
                ? "No display on this machine."
                : `${status.label} - nothing is running.`}
            </p>
          </div>
        )}

        <span className="absolute top-2.5 left-2.5 inline-flex h-5 items-center gap-1.5 rounded-full fill-secondary px-2 text-[10.5px] text-muted-foreground">
          <span
            aria-hidden="true"
            className={cn(
              "size-1.5 rounded-full",
              running && "animate-soft-pulse",
            )}
            style={{ backgroundColor: status.color }}
          />
          {status.label}
        </span>

        <IconButton
          size="sm"
          label="Open in Computers"
          onClick={open}
          className="absolute top-2 right-2"
        >
          <Maximize2 />
        </IconButton>
      </div>

      {/* The screen, without leaving the chat. "Take control" was once a
          toggle here that flipped a boolean and reached nothing; now there is
          a real control channel - the machine's own VNC - and the dialog
          carries the composer, so the person can drive and talk at once. */}
      <div className="grid grid-cols-2 gap-2">
        <Button
          variant="subtle"
          size="xs"
          disabled={!running}
          onClick={() => setScreen("control")}
        >
          <Hand />
          Take control
        </Button>
        <Button variant="subtle" size="xs" disabled={!running} onClick={() => setScreen("view")}>
          <Eye />
          View
        </Button>
      </div>
      <ComputerDialog
        computer={computer}
        thread={thread}
        mode={screen ?? "view"}
        open={screen !== null}
        onOpenChange={(next) => {
          if (!next) setScreen(null);
        }}
      />

      <div className="grid grid-cols-2 gap-x-3 gap-y-2.5">
        <Fact label="Machine" value={computer.name} />
        <Fact label="Provider" value={provider?.label ?? computer.provider} />
        <Fact label="Status" value={status.label} />
        <Fact
          label="Started"
          value={running && computer.startedAt ? relativeTime(computer.startedAt) : " - "}
        />
      </div>

      <div className="flex flex-col gap-2">
        <Meter label="CPU" value={computer.usage?.cpuPct ?? 0} tone={meterTone(computer.usage?.cpuPct)} />
        <Meter label="Memory" value={computer.usage?.memPct ?? 0} tone={meterTone(computer.usage?.memPct)} />
        <Meter label="Disk" value={computer.usage?.diskPct ?? 0} tone={meterTone(computer.usage?.diskPct)} />
      </div>
    </div>
  );
}

/** A provider that cannot measure something answers null, and an unmeasured
 *  meter must not read as a healthy one. */
function meterTone(pct) {
  if (pct == null) return "neutral";
  return pct > 85 ? "warning" : "neutral";
}

/* ── files ────────────────────────────────────────────────────────────────── */

function FilesSection({ files, agents }) {
  // The row that is open, held here rather than in the panel: the dialog is
  // about one file and nothing else in the panel needs to know which.
  const [openFile, setOpenFile] = React.useState(null);

  const author = (file) =>
    file?.agentId ? (agents.find((a) => a.id === file.agentId) ?? null) : null;

  if (!files.length) {
    return (
      <Empty>Nothing has been attached or written in this thread yet.</Empty>
    );
  }

  /**
   * Handed over, and merely touched.
   *
   * The panel used to be one list of every path a write tool had seen, which
   * for a turn that scaffolds a project is forty rows of intermediate work with
   * the two files worth opening lost in the middle of it. The agent says which
   * ones are the result - see the `present` tool - and those go first, under a
   * heading, with everything else still there below.
   *
   * The rest is kept rather than hidden. It is the honest record of what the
   * turn touched, and a panel that quietly omitted a file an agent wrote would
   * be worse than one that lists too many.
   */
  const presented = files.filter((file) => file.origin === "presented");
  const rest = files.filter((file) => file.origin !== "presented");

  const heading = (text, count) => (
    <p className="px-2 pb-1 pt-2 text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground/70">
      {text}
      <span className="ml-1 tabular-nums opacity-70">{count}</span>
    </p>
  );

  const rows = (list) => (
    <ul className="flex flex-col gap-0.5">
      {list.map((file) => {
        const wrote = author(file);
        const size = formatBytes(file.sizeBytes);
        const meta =
          file.origin === "attached"
            ? [size, wrote ? wrote.name : "You"].filter(Boolean).join(" · ")
            : [wrote?.name, "wrote"].filter(Boolean).join(" ") || "written";

        return (
          <li key={file.id}>
            <button
              type="button"
              title={file.path}
              onClick={() => setOpenFile(file)}
              className={cn(
                "group flex w-full items-center gap-2 rounded-xl px-2 py-1.5 text-left outline-none",
                "transition-colors duration-75 ease-out hover:fill-nav",
                "focus-visible:fill-nav",
              )}
            >
              <span className="grid size-7 shrink-0 place-items-center rounded-lg fill-whisper text-muted-foreground">
                <Icon
                  name={FILE_KIND_ICON[file.kind] ?? "File"}
                  className="size-3.5"
                />
              </span>
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-[12px] text-foreground">
                  {file.name}
                </span>
                <span className="truncate text-[10.5px] text-muted-foreground">
                  {meta}
                </span>
              </span>
              <span className="shrink-0 text-[10.5px] tabular-nums text-muted-foreground/70">
                {relativeTime(file.at)}
              </span>
            </button>
          </li>
        );
      })}
    </ul>
  );

  return (
    <>
      {presented.length ? (
        <>
          {heading("Handed over", presented.length)}
          {rows(presented)}
        </>
      ) : null}
      {rest.length ? (
        <>
          {presented.length ? heading("Also touched", rest.length) : null}
          {rows(rest)}
        </>
      ) : null}

      <ArtifactDialog
        artifact={openFile}
        onOpenChange={() => setOpenFile(null)}
        facts={
          openFile
            ? [
                openFile.origin === "attached"
                  ? ["Size", formatBytes(openFile.sizeBytes) ?? "Unknown"]
                  : ["Location", openFile.path],
                [
                  openFile.origin === "attached" ? "Attached by" : "Written by",
                  author(openFile)?.name ?? "You",
                ],
                ["When", relativeTime(openFile.at)],
              ]
            : []
        }
      />
    </>
  );
}

/* ── routines ─────────────────────────────────────────────────────────────── */

function RoutinesSection({ routines }) {
  const { setView, setActiveRoutineId } = useApp();

  if (!routines.length) {
    return (
      <Empty>
        This agent has no routines. A routine is work it runs without being
        asked.
      </Empty>
    );
  }

  return (
    <ul className="flex flex-col gap-0.5">
      {routines.map((routine) => (
        <li key={routine.id}>
          <button
            type="button"
            onClick={() => {
              setActiveRoutineId(routine.id);
              setView("routines");
            }}
            className={cn(
              "flex w-full items-center gap-2 rounded-xl px-2 py-1.5 text-left outline-none",
              "transition-colors duration-75 ease-out hover:fill-nav",
              "focus-visible:fill-nav",
            )}
          >
            <span
              className={cn(
                "grid size-7 shrink-0 place-items-center rounded-lg fill-whisper",
                routine.enabled ? "text-success-ink" : "text-muted-foreground",
              )}
            >
              {routine.enabled ? (
                <CalendarClock className="size-3.5" />
              ) : (
                <Pause className="size-3.5" />
              )}
            </span>
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate text-[12px] text-foreground">
                {routine.name}
              </span>
              <span className="truncate text-[10.5px] text-muted-foreground">
                {routine.enabled
                  ? (routine.schedule?.humanLabel ?? "On demand")
                  : "Paused"}
              </span>
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}

/* ── the panel ────────────────────────────────────────────────────────────── */

export function ChatSidePanel({ thread, agent, blocks, runs }) {
  const {
    agents,
    computers,
    routines,
    memories,
    chatPanelOpen,
    setChatPanelOpen,
    setActiveAgentId,
    setActiveRoutineId,
    setView,
    modelPrices,
    chatModels,
    defaultModelRef,
  } = useApp();

  const computer =
    computers.find((c) => c.id === thread?.computerAttached) ?? null;
  const files = React.useMemo(() => collectThreadFiles(blocks), [blocks]);
  const agentRoutines = routines.filter((r) => r.agentId === thread?.agentId);
  const agentMemories = memories.filter((m) => m.agentId === thread?.agentId);

  // The model that would actually answer, named the way the header names it:
  // a configured model beats an entry from the demo catalogue the agent was
  // seeded with, and it is looked up by ref, not by the bare id that only the
  // fixture used. Falling back to the workspace default, then to the seed.
  const model =
    chatModels.find((m) => m.ref === (agent?.model ?? defaultModelRef)) ??
    (chatModels.length
      ? chatModels.find((m) => m.ref === defaultModelRef) ?? chatModels[0]
      : MODELS.find((m) => m.id === agent?.model));
  const modelName = model?.label ?? model?.name ?? "Default";

  /**
   * What this conversation has cost.
   *
   * Summed per message rather than for the thread as a whole, because a thread
   * can change model halfway through and one price applied to the lot would be
   * wrong in whichever direction is least convenient. A message whose model
   * nobody has priced contributes its tokens and no dollars, and the panel says
   * so rather than showing a total that quietly excludes it.
   */
  const spend = React.useMemo(() => {
    let usage = null;
    let dollars = 0;
    let unpriced = 0;
    let helpers = 0;

    for (const message of blocks ?? []) {
      if (!message?.usage) continue;
      usage = addUsage(usage, message.usage);
      const { modelId } = parseModelRef(message.model ?? "");
      const cost = costOf(message.usage, priceFor(modelId, modelPrices));
      if (cost == null) unpriced += 1;
      else dollars += cost;
    }

    /**
     * And what the helpers spent, which is money too.
     *
     * A delegated run is a whole turn of its own - its own context, its own
     * tools, its own tokens - and none of it appeared here, because this only
     * ever added up the messages in the thread and a helper writes none. A
     * conversation that spawned eight subagents reported the cost of the
     * sentence that spawned them, which is the wrong number by an order of
     * magnitude and wrong in the direction that costs the user money.
     *
     * Priced by the helper's own model rather than the thread's: a run
     * delegated to a cheaper model is most of the reason to delegate, and
     * charging it at the parent's rate would hide exactly that.
     */
    for (const run of runs ?? []) {
      if (!run?.usage) continue;
      helpers += 1;
      usage = addUsage(usage, run.usage);
      const { modelId } = parseModelRef(run.model ?? "");
      const cost = costOf(run.usage, priceFor(modelId || run.model, modelPrices));
      if (cost == null) unpriced += 1;
      else dollars += cost;
    }

    return { usage, dollars: usage ? dollars : null, unpriced, helpers };
  }, [blocks, runs, modelPrices]);

  // How much of the input never had to be paid for at full price. On a long
  // thread this is the number that explains the cost, and without it a bill
  // that quietly doubled because caching stopped working looks like nothing.
  const cachedShare = React.useMemo(() => cacheHitRate(spend.usage), [spend.usage]);

  // Only while it is covering the transcript - a docked column is part of the
  // layout, and closing it on Escape there would be a surprise.
  React.useEffect(() => {
    if (!chatPanelOpen) return undefined;
    const onKeyDown = (e) => {
      if (e.key !== "Escape") return;
      if (window.matchMedia("(min-width: 64rem)").matches) return;
      setChatPanelOpen(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [chatPanelOpen, setChatPanelOpen]);

  const openAgent = () => {
    if (!agent) return;
    setActiveAgentId(agent.id);
    setView("agents");
  };

  // The dock decides whether this is on screen and how tall it is; this fills
  // whatever it is given.
  return (
    <div className="flex h-full min-h-0 w-full flex-col" aria-label="Thread details">
      <div className="flex h-full min-h-0 w-full flex-col">
        <header className="flex h-14 shrink-0 items-center gap-1 pr-2 pl-3.5">
          <span className="min-w-0 flex-1 truncate text-[12px] text-muted-foreground">
            {agent ? `${agent.name} · ${agent.role}` : "Thread"}
          </span>
          <IconButton size="md" label="Open agent" onClick={openAgent}>
            <Settings2 />
          </IconButton>
          <IconButton
            size="md"
            label="Close panel"
            onClick={() => setChatPanelOpen(false)}
          >
            <X />
          </IconButton>
        </header>

        <ScrollArea fade className="min-h-0 flex-1 px-3.5 pb-4">
          <div className="flex flex-col gap-3">
            <Section id="computer" title="Computer">
              <ComputerSection computer={computer} agent={agent} thread={thread} />
            </Section>

            <Section id="files" title="Files" count={files.length || undefined}>
              <FilesSection files={files} agents={agents} />
            </Section>

            <Section
              id="routines"
              title="Routines"
              count={agentRoutines.length || undefined}
              action={
                <IconButton
                  size="sm"
                  label="New routine"
                  onClick={() => {
                    setActiveRoutineId(null);
                    setView("routines");
                  }}
                >
                  <Plus />
                </IconButton>
              }
            >
              <RoutinesSection routines={agentRoutines} />
            </Section>

            <Section id="session" title="Session" defaultOpen={false}>
              <div className="grid grid-cols-2 gap-x-3 gap-y-2.5">
                <Fact label="Model" value={modelName} />
                <Fact label="Messages" value={String(blocks.length)} />
                {/* Only when there were any. A row reading "Helpers 0" on every
                    ordinary conversation is a line of noise about a feature
                    that was not used. */}
                {spend.helpers ? (
                  <Fact
                    label="Helpers"
                    value={String(spend.helpers)}
                    title="Delegated runs whose tokens and cost are counted in the totals below."
                  />
                ) : null}
                <Fact label="Memories" value={String(agentMemories.length)} />
                <Fact
                  label="Tokens"
                  value={spend.usage ? formatTokens(totalTokens(spend.usage)) : "-"}
                  title={
                    spend.usage
                      ? `${spend.usage.input.toLocaleString()} in, ` +
                        `${spend.usage.output.toLocaleString()} out, over ` +
                        `${spend.usage.requests} request${spend.usage.requests === 1 ? "" : "s"}` +
                        (spend.helpers
                          ? `, including ${spend.helpers} delegated run${spend.helpers === 1 ? "" : "s"}`
                          : "")
                      : "No provider in this thread has reported a token count yet."
                  }
                />
                <Fact
                  label="Cost"
                  value={
                    spend.unpriced && !spend.dollars
                      ? "No price"
                      : formatCost(spend.dollars) ?? "-"
                  }
                  title={
                    spend.unpriced
                      ? `${spend.unpriced} message${spend.unpriced === 1 ? "" : "s"} used a model ` +
                        `with no price. Set one under Settings, Providers.`
                      : `Estimated from list prices as of ${PRICES_AS_OF}.`
                  }
                />
                <Fact
                  label="Cached"
                  value={cachedShare == null ? "-" : `${Math.round(cachedShare * 100)}%`}
                  title={
                    cachedShare == null
                      ? "This provider has not reported a cached read. Prompt caching is spoken natively to Anthropic; through an OpenAI-shaped gateway it is whatever that gateway does on its own."
                      : `${spend.usage.cached.toLocaleString()} of ${spend.usage.input.toLocaleString()} input tokens were read from the cache at a tenth of the price, ` +
                        `and ${(spend.usage.cacheWrite ?? 0).toLocaleString()} were written into it.`
                  }
                />
                <Fact
                  label="Last active"
                  value={relativeTime(thread.updatedAt)}
                  title={absoluteTime(thread.updatedAt)}
                />
              </div>

              <div className="mt-3 flex flex-col gap-1.5">
                <Button
                  variant="subtle"
                  size="xs"
                  className="w-full"
                  onClick={openAgent}
                >
                  <Sparkles />
                  Agent settings
                </Button>
                <Button
                  variant="subtle"
                  size="xs"
                  className="w-full"
                  onClick={() => setView("memory")}
                >
                  <Brain />
                  Browse memory
                </Button>
              </div>
            </Section>
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}
