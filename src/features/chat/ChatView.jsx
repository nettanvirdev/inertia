import * as React from "react";
import {
  ArrowDown,
  Download,
  MessageSquarePlus,
  Monitor,
  MoreHorizontal,
  Pencil,
  Globe,
  Phone,
  Pin,
  PinOff,
  SquareTerminal,
  Trash2,
  Users,
  X,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { GUTTER } from "@/components/layout/View";
import { MODELS } from "@/data";
import { useApp } from "@/lib/store";
import { appearanceOf } from "@/lib/appearance";
import { copyText } from "@/lib/clipboard";
import { isThreadWorking, todosOf } from "@shared/transcript";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Dialog, DialogTitle, DialogFooter } from "@/components/ui/dialog";
import { DropdownMenu, MenuItem, MenuSeparator } from "@/components/ui/dropdown-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useToast } from "@/components/ui/toast";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { Composer } from "@/features/chat/Composer";
import { MessageBubble } from "@/features/chat/MessageBubble";
import { WINDOW_SIZE, windowOf } from "@/features/chat/transcript-window";
import { PermissionPrompt } from "@/features/chat/PermissionPrompt";
import { QuestionPrompt } from "@/features/chat/QuestionPrompt";
import { usePrompts } from "@/features/chat/use-prompts";
import { StarterPrompts } from "@/features/chat/StarterPrompts";
import { ChatSidePanel } from "@/features/chat/SidePanel";
import { SideDock } from "@/features/chat/SideDock";
import { WebPane } from "@/features/chat/WebPane";
import { ShellPane } from "@/features/chat/ShellPane";
import { isTerminalAvailable, terminal } from "@/lib/terminal";
import { isPreviewAvailable, preview } from "@/lib/preview";
import { revealPane } from "@/features/chat/pane-reveal";
import { CrewPanel } from "@/features/chat/CrewPanel";
import { ContextMeter } from "@/features/chat/ContextMeter";
import { PlanButton } from "@/features/chat/PlanButton";
import { ToolCallCard } from "@/features/chat/ToolCallCard";
import { RunFolder } from "@/features/chat/markdown/run-folder.js";
import { ProjectSetupOffer } from "@/features/chat/ProjectSetupOffer";
import { useAutoSpeak } from "@/hooks/use-auto-speak";
import { useNow } from "@/hooks/use-now";
import { CallView } from "@/features/chat/CallView";
import { voice as voiceClient } from "@/lib/voice";
import { useCrew } from "@/lib/crew";
import { isWorking } from "@shared/crew";
import { FloorNote, RoomStrip } from "@/features/chat/RoomStrip";
import { speakingAgent } from "@/features/chat/room.js";

/**
 * The chat screen: thread list on the left, transcript on the right.
 *
 * Surface choice - the rail is `sidebar`, the transcript is `background`, and
 * the list pane sits on `card-darker`, one rung below both. Per
 * patterns.surface_step_rule that reads as a distinct plane in light AND dark
 * without a single stroke; putting the list on `sidebar` would have merged it
 * into the rail in dark, where the two tokens are identical.
 */

/**
 * How long a freshly opened conversation is held at its end.
 *
 * Long enough for code blocks and images to finish measuring - they all add
 * height below the position just set - and short enough that it cannot be
 * mistaken for the view refusing to move.
 */
const SETTLE_MS = 600;

const NEAR_BOTTOM_PX = 120;
const NO_BLOCKS = [];

/* ── the view ─────────────────────────────────────────────────────────────── */

export function ChatView() {
  const {
    agents,
    threads,
    messages,
    activeThreadId,
    startingAgentId,
    createThread,
    sendMessage,
    retryMessage,
    togglePinThread,
    renameThread,
    deleteThread,
    deleteLastExchange,
    setView,
    chatPanelOpen,
    toggleChatPanel,
    chatModels,
    defaultModelRef,
    modelPrices,
    user,
    openSettings,
  } = useApp();
  const { toast } = useToast();

  // Every message caps itself at `--chat-measure`, so the column only has to
  // get out of the way. It does that for exactly one choice: "Full width" is
  // the one setting that means "wider than the reading column", and leaving
  // --content-max in place there would make the option do nothing.
  const fullWidth = appearanceOf(user?.preferences).messageWidth === "full";
  const columnClass = fullWidth ? "max-w-full" : "max-w-[var(--content-max)]";

  const [renaming, setRenaming] = React.useState(null); // { id, title }
  const [deleting, setDeleting] = React.useState(null); // thread
  const [atBottom, setAtBottom] = React.useState(true);

  /**
   * The runs this conversation has started, and whether the panel is open.
   *
   * Subscribed for every thread rather than only while the panel is open,
   * because the icon that opens the panel is the thing that has to know: an
   * icon you have to open a panel to discover is not an indicator.
   */
  const crewRuns = useCrew(activeThreadId);
  // Both live in the dock now and stack, so opening one no longer closes the
  // other - which was only ever a rule about a slot they had to share.
  // Read once. Whether this build has a backend behind it does not change
  // while the window is open.
  const previewAvailable = isPreviewAvailable();
  const terminalAvailable = isTerminalAvailable();
  const [crewOpen, setCrewOpen] = React.useState(false);
  // The browser beside the conversation. Not persisted: a native view restored
  // over a window that has not finished laying out lands in the wrong place,
  // and opening it is one click.
  const [webOpen, setWebOpen] = React.useState(false);
  // The shell for this conversation. It outlives the pane - closing this does
  // not kill a running build - so the toggle is only about whether it is on
  // screen.
  const [shellOpen, setShellOpen] = React.useState(false);
  // The dot means "working", not "unfinished". A paused run is neither, and
  // saying "one agent working" over one somebody paused is a lie a glance
  // cannot catch.
  const activeRuns = crewRuns.filter((run) => isWorking(run.status)).length;

  /**
   * The agent touched a browser or a terminal: show it.
   *
   * Main asks; this decides what asking means. The dock has to be OPEN before
   * anything else can front a panel inside it, and that state lives here - so
   * this listener opens it and then passes the request on to the two
   * components that own the rest. See `pane-reveal.js`.
   *
   * Guarded by the conversation, because a tool running in one thread must not
   * pull open the panels of another that happens to be on screen.
   */
  React.useEffect(() => {
    if (!activeThreadId) return undefined;
    const show = (event, dock) => {
      if (event?.type !== "reveal" || event.threadId !== activeThreadId) return;
      if (dock === "web") setWebOpen(true);
      else setShellOpen(true);
      revealPane({ dock, tabId: event.id, threadId: event.threadId });
    };
    const offWeb = previewAvailable ? preview.onEvent((event) => show(event, "web")) : () => {};
    const offShell = terminalAvailable
      ? terminal.onEvent((event) => show(event, "shell"))
      : () => {};
    return () => {
      offWeb?.();
      offShell?.();
    };
  }, [activeThreadId, previewAvailable, terminalAvailable]);

  const scrollRef = React.useRef(null);
  const thread = threads.find((t) => t.id === activeThreadId) ?? null;
  const agent = agents.find((b) => b.id === thread?.agentId) ?? null;
  const threadId = thread?.id ?? null;
  // A stable empty list, so a thread with nothing in it does not hand every
  // memo below a new array on every render.
  const blocks = React.useMemo(
    () => (threadId ? (messages[threadId] ?? NO_BLOCKS) : NO_BLOCKS),
    [threadId, messages]
  );
  /**
   * Whose conversation this is right now.
   *
   * Outside a group this is the thread's agent and always was. Inside one it
   * moves: an agent that is handed the conversation puts its name and face at
   * the top of it, which is the only way a person can tell, at a glance, who
   * they are talking to.
   */
  /**
   * Whether this conversation has begun.
   *
   * Not "has a thread" - a new chat has one of those the moment it is opened.
   * This is whether anybody has said anything in it, which is what decides
   * whether the controls that act ON a conversation have something to act on.
   */
  const started = blocks.length > 0;

  const speaking = React.useMemo(
    () => speakingAgent({ thread, messages: blocks, agents }) ?? agent,
    [thread, blocks, agents, agent]
  );

  /**
   * How much of this conversation is in the document.
   *
   * Reset per thread, because "I opened the last two hundred messages of that
   * one" is not a fact about the next one. See `transcript-window.js` for why
   * this is a window rather than a virtualised list.
   */
  const [shown, setShown] = React.useState(WINDOW_SIZE);
  React.useEffect(() => {
    setShown(WINDOW_SIZE);
  }, [threadId]);
  const { visible, hidden } = React.useMemo(() => windowOf(blocks, shown), [blocks, shown]);

  const { prompts, replyPermission, answerQuestion, dismissQuestion } = usePrompts(thread?.id);
  // The task list as it stands: the newest one the agent wrote in this thread,
  // patched in place as the turn runs, so the header follows the work live.
  const todos = React.useMemo(() => todosOf(blocks), [blocks]);
  // null when there is no call. A live call carries when it started and
  // whether its sheet is showing, because ending it and hiding it are
  // different things and the first version only had the one.
  const [call, setCall] = React.useState(null);
  const [voiceReady, setVoiceReady] = React.useState(false);
  useAutoSpeak(blocks, thread?.id ?? null, { suspended: Boolean(call) });

  // Whether a key is stored, so the Call button can route someone to the place
  // that fixes it rather than opening a call that cannot hear or answer.
  React.useEffect(() => {
    let alive = true;
    voiceClient
      .configured()
      .then((value) => alive && setVoiceReady(Boolean(value)))
      .catch(() => alive && setVoiceReady(false));
    return () => {
      alive = false;
    };
  }, []);

  // A call belongs to the conversation it was started in.
  React.useEffect(() => {
    setCall(null);
  }, [thread?.id]);

  /* ── scroll pinning ───────────────────────────────────────────────────── */
  const scrollToBottom = React.useCallback((behavior = "smooth") => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior });
  }, []);

  /**
   * Put the earlier messages back, without moving what is on screen.
   *
   * Content inserted above the viewport pushes everything down by its own
   * height, so the reader would be looking at a different part of the
   * conversation than the one they were reading. The distance from the bottom
   * is the invariant to hold: it is unchanged by anything added above, and
   * restoring it after the paint puts the same pixels back under their eyes.
   */
  const showEarlier = React.useCallback(() => {
    const el = scrollRef.current;
    const fromBottom = el ? el.scrollHeight - el.scrollTop : 0;
    setShown((count) => count + WINDOW_SIZE);
    if (!el) return;
    requestAnimationFrame(() => {
      // `behavior: "auto"`, explicitly: the pane scrolls smoothly by default
      // now, and a smooth restore is a visible slide to the place the reader
      // never left.
      el.scrollTo({ top: el.scrollHeight - fromBottom, behavior: "auto" });
    });
  }, []);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    setAtBottom(el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX);
  };

  // Follow new output only when the reader is already at the bottom - yanking
  // someone out of scrollback to show a token is the rudest thing a chat does.
  React.useEffect(() => {
    if (atBottom) scrollToBottom("auto");
  }, [blocks.length, blocks[blocks.length - 1]?.content, atBottom, scrollToBottom]);

  /**
   * A question always comes into view, wherever the reader was.
   *
   * The one exception to not yanking somebody out of scrollback, and it earns
   * it: a permission card is a tool call suspended mid-execution, so nothing
   * else in this conversation happens until it is answered. It renders at the
   * end of the transcript, and following new output only while already at the
   * bottom meant it appeared under the fold - half a card behind the composer,
   * with its buttons off screen. What that looked like was a conversation that
   * had silently stopped, and the way people found the card was by leaving the
   * screen and coming back, which remounts and scrolls to the end.
   */
  const asked = prompts.length;
  const lastAsked = React.useRef(asked);
  React.useEffect(() => {
    const grew = asked > lastAsked.current;
    lastAsked.current = asked;
    if (!grew) return;
    setAtBottom(true);
    scrollToBottom();
  }, [asked, scrollToBottom]);

  /**
   * A conversation opens at its end, and does not travel there.
   *
   * Three things had to change for that, and each of them was on its own
   * enough to make a long thread scroll past under the reader on every open.
   *
   * **Before the paint, not after.** This was a `useEffect`, which runs once
   * the browser has already drawn - so the first frame of every conversation
   * was its oldest message, and the jump to the end was a correction the
   * reader could see. A layout effect writes the scroll position while the
   * frame is still being built.
   *
   * **`scrollTop`, not `scrollTo`.** The viewport carries `scroll-smooth`.
   * A layout effect that asked to be moved would have been animated, which is
   * the travelling this is meant to stop; `scrollTop` set inside a layout
   * effect lands before there is anything to animate from.
   *
   * **And then again, until it settles.** The height at that moment is not
   * the final height: code blocks, images and markdown all measure after the
   * first layout, and every one of them adds pixels below the position just
   * set. So the end is held for a moment - long enough for the content to
   * finish arriving, short enough that it can never fight a reader who has
   * started scrolling, which is what `atBottom` going false means.
   */
  React.useLayoutEffect(() => {
    setAtBottom(true);
    const el = scrollRef.current;
    if (!el) return undefined;

    const pin = () => {
      el.scrollTop = el.scrollHeight;
    };
    pin();

    const observer = new ResizeObserver(pin);
    // The content, not the viewport: the viewport's size is not what changes
    // when a code block finishes laying itself out.
    for (const child of el.children) observer.observe(child);
    const until = setTimeout(() => observer.disconnect(), SETTLE_MS);

    return () => {
      observer.disconnect();
      clearTimeout(until);
    };
  }, [activeThreadId]);

  // Ticks only while a call is live, so an idle thread does no work for a
  // clock nobody is looking at.
  const callTick = useNow(call ? 1000 : 0);
  const callElapsed = React.useMemo(() => {
    if (!call) return "";
    void callTick;
    const seconds = Math.max(0, Math.round((Date.now() - call.startedAt) / 1000));
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  }, [call, callTick]);

  // Everything that means "this conversation has not finished": a reply still
  // arriving, a call still running inside one that has, and a question waiting
  // on the person. The rule is in `shared/transcript` with its tests, because
  // getting it wrong is what put a Send button and a timestamp on a turn that
  // was suspended over a permission card.
  const streaming = React.useMemo(() => isThreadWorking(blocks, prompts), [blocks, prompts]);

  /* ── thread actions ───────────────────────────────────────────────────── */
  const exportTranscript = () => {
    const text = blocks
      .map((b) =>
        b.type === "tool"
          ? `> [${b.tool}] ${b.title}`
          : `**${b.role === "user" ? "You" : (agents.find((x) => x.id === b.agentId)?.name ?? "Agent")}:** ${b.content}`
      )
      .join("\n\n");
    void copyText(`# ${thread?.title ?? "Thread"}\n\n${text}`).then((done) =>
      toast(
        done
          ? { variant: "success", title: "Transcript copied as Markdown" }
          : {
              variant: "warning",
              title: "Could not copy the transcript",
              description: "This window has no access to the clipboard.",
            }
      )
    );
  };

  const confirmDelete = () => {
    if (deleting) deleteThread(deleting.id);
    setDeleting(null);
    toast({ title: "Thread deleted" });
  };

  const commitRename = () => {
    if (renaming?.title.trim()) renameThread(renaming.id, renaming.title.trim());
    setRenaming(null);
  };

  // The header names the model that would actually answer, which once a
  // provider exists is a configured one rather than an entry from the demo
  // catalogue the agent was seeded with.
  const model =
    chatModels.find((m) => m.ref === (agent?.model ?? defaultModelRef)) ??
    (chatModels.length
      ? chatModels.find((m) => m.ref === defaultModelRef) ?? chatModels[0]
      : MODELS.find((m) => m.id === agent?.model));

  // Where a Run button in a code block runs. The worktree this conversation
  // entered wins over the agent's own folder, for the same reason a turn's
  // does: while a thread is in a worktree, that is where its work is.
  const runFolder = thread?.worktree?.path ?? agent?.cwd ?? null;

  return (
    <RunFolder.Provider value={runFolder}>
      <div className="relative flex min-h-0 flex-1 overflow-hidden">
        {/* ── transcript ─────────────────────────────────────────────────── */}
        <section className="flex min-w-0 flex-1 flex-col">
          {thread ? (
            <>
              <header className={cn("flex h-14 shrink-0 items-center gap-2", GUTTER)}>
                {speaking ? <AgentAvatar agent={speaking} size="sm" showStatus /> : null}
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate text-sm font-medium text-foreground" title={thread.title}>
                    {speaking?.name ?? "Thread"}
                  </span>
                  {/* Who else is in here. Straight after the name, because the
                      question it answers is the one a reply under an
                      unexpected name provokes. */}
                  <RoomStrip room={thread.room} agents={agents} speaking={speaking?.id} />
                  {model ? (
                    <span className="hidden shrink-0 rounded-full fill-control px-2 py-0.5 text-[11px] text-muted-foreground md:inline">
                      {model.name ?? model.label}
                    </span>
                  ) : null}
                  {thread.computerAttached ? (
                    <span className="hidden shrink-0 items-center gap-1 rounded-full fill-control px-2 py-0.5 text-[11px] text-muted-foreground lg:inline-flex">
                      <Monitor className="size-3" aria-hidden="true" />
                      Computer attached
                    </span>
                  ) : null}
                  {/* Only once a turn has reported it: a gauge with nothing
                      behind it would read as an empty conversation. */}
                  {thread.context?.window ? <ContextMeter context={thread.context} /> : null}
                </div>

                <div className="ml-auto flex shrink-0 items-center gap-1">
                  {/* Only when there is a plan. A permanently present control
                      that is empty most of the time teaches people to ignore it,
                      and this one is worth looking at exactly when it appears. */}
                  {todos.length ? <PlanButton todos={todos} /> : null}
                  {/* Not on an empty thread. There is nobody to call yet: a
                      conversation with no messages in it has had no turn, so
                      the agent has no context to be spoken to about, and a
                      call placed here is a microphone open to a blank page.
                      The control comes back with the first message.

                      Never disabled when voice is unconfigured, though. A
                      control that is greyed out tells someone they cannot, and
                      not what to do about it; this one opens the pane that
                      fixes it. It is also the only thing on screen that says
                      voice exists at all. */}
                  {call ? (
                    <>
                      {/* While a call is live this stops being a way to start one
                          and becomes the call itself: the body brings the sheet
                          back, the X ends it. A backgrounded call with no visible
                          way to end it is a microphone nobody can find. */}
                      <button
                        type="button"
                        onClick={() => setCall((c) => c && { ...c, minimized: false })}
                        className={cn(
                          "flex h-8 items-center gap-1.5 rounded-full pl-2.5 pr-3 text-[13px] outline-none",
                          "fill-secondary text-foreground transition-colors duration-150 ease-out",
                          "hover:fill-control-hover focus-visible:fill-control-hover"
                        )}
                        title={call.minimized ? "Back to the call" : "The call is open"}
                      >
                        <Phone className="size-3.5 shrink-0" aria-hidden="true" />
                        <span className="tabular-nums">{callElapsed}</span>
                      </button>
                      <IconButton size="lg" label="Hang up" onClick={() => setCall(null)}>
                        <X />
                      </IconButton>
                    </>
                  ) : started ? (
                    <IconButton
                      size="lg"
                      label={voiceReady ? "Call this agent" : "Set up voice to call"}
                      onClick={() => {
                        if (!voiceReady) {
                          openSettings("voice");
                          return;
                        }
                        setCall({ startedAt: Date.now(), minimized: false });
                      }}
                    >
                      <Phone />
                    </IconButton>
                  ) : null}
                  {/* Only while there is a team. An icon that is always there and
                      usually empty teaches people not to look at it, and the one
                      thing this has to do is be noticed the moment work is handed
                      out. It stays after everyone has finished, because looking at
                      what a subagent did once it is over is half the point. */}
                  {crewRuns.length ? (
                    <IconButton
                      size="lg"
                      label={
                        activeRuns
                          ? `${activeRuns} agent${activeRuns === 1 ? "" : "s"} working`
                          : "Agents in this conversation"
                      }
                      active={crewOpen}
                      onClick={() => setCrewOpen((open) => !open)}
                    >
                      <span className="relative inline-flex">
                        <Users />
                        {activeRuns ? (
                          <span className="bg-primary absolute -top-0.5 -right-1 size-2 rounded-full" />
                        ) : null}
                      </span>
                    </IconButton>
                  ) : null}
                  {/* A browser, on this machine, with localhost reachable and
                      whatever you are signed in to. The same one the agent's
                      `browser_*` tools drive, which is the point: what it
                      reports and what you can see are the same page. */}
                  {previewAvailable ? (
                    <IconButton
                      size="lg"
                      label={webOpen ? "Hide the browser" : "Open a browser"}
                      active={webOpen}
                      onClick={() => setWebOpen((open) => !open)}
                    >
                      <Globe />
                    </IconButton>
                  ) : null}
                  {/* A shell that stays where you left it, in the folder this
                      conversation is working in. The agent can read it and
                      cannot type into it - see src-tauri/src/terminal.rs for
                      why. */}
                  {terminalAvailable ? (
                    <IconButton
                      size="lg"
                      label={shellOpen ? "Hide the terminal" : "Open a terminal"}
                      active={shellOpen}
                      onClick={() => setShellOpen((open) => !open)}
                    >
                      <SquareTerminal />
                    </IconButton>
                  ) : null}
                  <IconButton
                    size="lg"
                    label={chatPanelOpen ? "Hide thread details" : "Show thread details"}
                    active={chatPanelOpen}
                    onClick={toggleChatPanel}
                  >
                    <Monitor />
                  </IconButton>
                  <DropdownMenu
                    align="end"
                    trigger={
                      <IconButton size="lg" label="Thread actions">
                        <MoreHorizontal />
                      </IconButton>
                    }
                  >
                    <MenuItem
                      icon={Pencil}
                      onSelect={() => setRenaming({ id: thread.id, title: thread.title })}
                    >
                      Rename
                    </MenuItem>
                    <MenuItem
                      icon={thread.pinned ? PinOff : Pin}
                      onSelect={() => togglePinThread(thread.id)}
                    >
                      {thread.pinned ? "Unpin thread" : "Pin thread"}
                    </MenuItem>
                    <MenuItem icon={Monitor} onSelect={() => setView("computers")}>
                      Attach computer
                    </MenuItem>
                    <MenuItem icon={Download} onSelect={exportTranscript}>
                      Export transcript
                    </MenuItem>
                    <MenuSeparator />
                    <MenuItem
                      icon={Trash2}
                      danger
                      disabled={!blocks.length}
                      onSelect={() => {
                        deleteLastExchange(thread.id);
                        toast({ title: "Last exchange removed" });
                      }}
                    >
                      Delete last
                    </MenuItem>
                  </DropdownMenu>
                </div>
              </header>

              {blocks.length ? (
                <>
                  <div className="relative flex min-h-0 flex-1 flex-col">
                    <ScrollArea viewportRef={scrollRef} onScroll={onScroll} className="flex-1">
                      <div className={cn("mx-auto flex min-h-full flex-col justify-end px-6 py-4", columnClass)}>
                        {/* The conversation before the window. A button rather
                            than an automatic load on scroll: loading on scroll
                            means the reader who is dragging the bar upward gets
                            content inserted under their thumb, and the bar
                            never reaches the top. */}
                        {hidden ? (
                          <div className="mb-2 flex justify-center">
                            <Button variant="ghost" size="xs" onClick={showEarlier}>
                              {`Show ${Math.min(hidden, WINDOW_SIZE)} earlier ${
                                Math.min(hidden, WINDOW_SIZE) === 1 ? "message" : "messages"
                              }`}
                            </Button>
                          </div>
                        ) : null}
                        {visible.map((block, index) =>
                          block.type === "tool" ? (
                            <ToolCallCard key={block.id} block={block} />
                          ) : (
                            <MessageBubble
                              key={block.id}
                              message={block}
                              agent={agents.find((b) => b.id === (block.agentId ?? thread.agentId))}
                              isLast={index === visible.length - 1}
                              threadId={activeThreadId}
                              retryMessage={retryMessage}
                              preferences={user?.preferences}
                            />
                          )
                        )}

                        {/* A tool that is waiting on a person is suspended, not
                            finished, so its question belongs at the end of the
                            transcript rather than in a modal: the reader needs to
                            scroll up and see what led here before deciding. */}
                        {prompts.map((prompt) =>
                          prompt.channel === "permission" ? (
                            <PermissionPrompt
                              key={prompt.question.id}
                              question={prompt.question}
                              onReply={replyPermission}
                            />
                          ) : (
                            <QuestionPrompt
                              key={prompt.question.id}
                              question={prompt.question}
                              onAnswer={answerQuestion}
                              onDismiss={dismissQuestion}
                            />
                          )
                        )}
                      </div>
                    </ScrollArea>

                    {!atBottom ? (
                      <button
                        type="button"
                        onClick={() => {
                          setAtBottom(true);
                          scrollToBottom();
                        }}
                        className={cn(
                          "absolute bottom-3 left-1/2 flex h-8 -translate-x-1/2 items-center gap-1.5 rounded-full",
                          "bg-foreground px-3 text-[13px] text-background outline-none animate-fade-in",
                          "transition-opacity duration-150 ease-out hover:opacity-90",
                          "focus-visible:opacity-90"
                        )}
                      >
                        <ArrowDown className="size-3.5" />
                        Jump to latest
                      </button>
                    ) : null}
                  </div>

                  {call ? (
                    <CallView
                      agent={agent}
                      messages={blocks}
                      streaming={streaming}
                      minimized={call.minimized}
                      onSend={(text) => {
                        setAtBottom(true);
                        sendMessage(thread.id, text);
                      }}
                      onMinimize={() => setCall((c) => c && { ...c, minimized: true })}
                      onClose={() => setCall(null)}
                    />
                  ) : null}

                  <div className={cn("mx-auto w-full px-6 pb-4 pt-1", columnClass)}>
                    {/* Above the composer rather than in the transcript: it is
                        about the folder, not about anything that was said. */}
                    <ProjectSetupOffer cwd={runFolder} />
                    {/* Only when the agents have talked among themselves long
                        enough to be stopped. The limit is there because the
                        bill is real, and it only works if the person is told
                        where they are about to type. */}
                    <FloorNote room={thread.room} />
                    <Composer
                      threadId={thread.id}
                      agentId={thread.agentId}
                      streaming={streaming}
                      onSend={(text, options) => {
                        setAtBottom(true);
                        sendMessage(thread.id, text, options);
                      }}
                    />
                  </div>
                </>
              ) : (
                /* An empty thread has nothing to anchor the composer to, so
                   docking it at the bottom leaves a tall column of nothing above
                   it and the greeting stranded at the top. Here the greeting,
                   the starters and the composer are ONE centred group - the
                   composer is the subject of the screen. It goes back to the
                   bottom edge the moment a transcript exists to hold it there.
                   my-auto (not justify-center) so a short window scrolls
                   instead of clipping the group. */
                <div className="flex min-h-0 flex-1 flex-col overflow-y-auto no-scrollbar px-6 py-6">
                  <div className="mx-auto my-auto flex w-full max-w-[var(--content-max)] shrink-0 flex-col gap-4">
                    <div className="flex flex-col gap-1">
                      <p className="text-lg font-medium leading-snug text-heading">
                        {agent ? `${agent.name} is ready.` : "Ready when you are."}
                      </p>
                      <p className="text-[13px] leading-relaxed text-muted-foreground">
                        {agent?.role ?? "Start a conversation."}
                      </p>
                    </div>

                    <StarterPrompts agent={agent} onPick={(text) => sendMessage(thread.id, text)} />

                    <Composer
                      threadId={thread.id}
                      agentId={thread.agentId}
                      streaming={streaming}
                      onSend={(text, options) => {
                        setAtBottom(true);
                        sendMessage(thread.id, text, options);
                      }}
                    />
                  </div>
                </div>
              )}
            </>
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-1">
              <img
                src="./assets/logo-256.png"
                alt=""
                aria-hidden="true"
                className="size-14 opacity-90"
              />
              <EmptyState
                title="No conversation open"
                description="Pick a thread on the left, or start a new one with any of your teammates."
                action={
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={() => createThread(startingAgentId)}
                  >
                    <MessageSquarePlus />
                    New chat
                  </Button>
                }
              />
            </div>
          )}
        </section>

        {/* Only ever beside a live thread: with no conversation there is no
            computer, no files and no session to describe. */}
        {/* One column, whatever is open stacked inside it. Only beside a live
            thread: with no conversation there is no computer, no files and no
            helpers to describe. */}
        {thread ? (
          <SideDock
            sections={[
              {
                id: "web",
                label: "Browser",
                icon: "Globe",
                open: webOpen,
                node: <WebPane paneId={thread.id} onClose={() => setWebOpen(false)} />,
              },
              {
                id: "shell",
                label: "Terminal",
                icon: "SquareTerminal",
                open: shellOpen,
                node: (
                  <ShellPane
                    paneId={thread.id}
                    cwd={runFolder}
                    onClose={() => setShellOpen(false)}
                  />
                ),
              },
              {
                id: "thread",
                label: "Details",
                icon: "Info",
                open: chatPanelOpen,
                node: (
                  <ChatSidePanel thread={thread} agent={agent} blocks={blocks} runs={crewRuns} />
                ),
              },
              {
                id: "agents",
                label: "Agents",
                icon: "Users",
                open: crewOpen,
                node: (
                  <CrewPanel
                    open={crewOpen}
                    onOpenChange={setCrewOpen}
                    conversationId={activeThreadId}
                    runs={crewRuns}
                    prices={modelPrices}
                  />
                ),
              },
            ]}
          />
        ) : null}

        {/* ── overlays ───────────────────────────────────────────────────── */}
        <Dialog
          open={Boolean(renaming)}
          onOpenChange={(next) => !next && setRenaming(null)}
          size="sm"
          ariaLabel="Rename thread"
        >
          <DialogTitle>Rename thread</DialogTitle>
          <div className="mt-4">
            <Input
              autoFocus
              size="sm"
              value={renaming?.title ?? ""}
              aria-label="Thread title"
              onChange={(e) => setRenaming((r) => ({ ...r, title: e.target.value }))}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitRename();
              }}
            />
          </div>
          <DialogFooter>
            <Button variant="secondary" size="pill" onClick={() => setRenaming(null)}>
              Cancel
            </Button>
            <Button variant="primary" size="pill" onClick={commitRename}>
              Save
            </Button>
          </DialogFooter>
        </Dialog>

        <ConfirmDialog
          open={Boolean(deleting)}
          onOpenChange={(next) => !next && setDeleting(null)}
          destructive
          title="Delete this thread?"
          description={
            deleting
              ? `“${deleting.title}” and its ${deleting.messageCount ?? 0} messages will be removed. This cannot be undone.`
              : undefined
          }
          confirmLabel="Delete"
          onConfirm={confirmDelete}
        />
      </div>
    </RunFolder.Provider>
  );
}
