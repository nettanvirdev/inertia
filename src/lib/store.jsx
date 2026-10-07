import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { PREF, readPref, usePersistentState, writePref } from "@/lib/persist";
import { useWorkspacePersistence } from "@/lib/workspace-store";
import { useChatRuntime } from "@/lib/chat-runtime";
import {
  activeTurns,
  turnRecord,
  applyEvent,
  attachTurn,
  isAgentAvailable,
  runTurn,
  summarize,
  textOf,
  toHistory,
  tools as agentTools,
} from "@/lib/agent";
import { allModels } from "@shared/providers";
import { DEFAULT_MODE, isMode } from "@shared/modes";
import { DEFAULT_APPROVAL, isApproval } from "@shared/approval";
import { forgetConversation } from "@/lib/crew";
import { mentionSpecs, mentionedIds } from "@/features/chat/mentions";
import { cutScriptParts, isPass } from "@/features/chat/self-label";
import { defaultAgent, isPaused, pausedReason } from "@shared/agents";
import { totalTokens } from "@shared/usage";
import { titleSource } from "@shared/title";
import { areComputersAvailable, computers as machines } from "@/lib/computers";
import { isSchedulerAvailable, routines as scheduler } from "@/lib/routines";
import { defaultRules } from "@shared/tools";
import { useToast } from "@/components/ui/toast";
import { ANY } from "@shared/permission";
import { PREFERENCE_DEFAULTS } from "@/lib/appearance";
import { CURRENT_USER } from "@/data";

const AppContext = createContext(null);

/**
 * A local id that is new, including after a reload.
 *
 * The counter alone was not. It restarted at one every time the window loaded,
 * so the first thread created after a restart was `thr-local-1` - which is what
 * the first thread of the *previous* session was called, and that one has been
 * restored from the workspace and is sitting in the list. React saw two
 * children with the same key and rendered one of them, and the user watched a
 * new chat overwrite an old one.
 *
 * The timestamp in base 36 is what makes it unique across sessions; the counter
 * is still there because two ids can be asked for inside the same millisecond,
 * and a thread and its first message routinely are.
 */
let seq = 0;
const uid = (prefix) => `${prefix}-local-${Date.now().toString(36)}-${++seq}`;

/** The views the shell can restore into. Kept here rather than imported from
 *  App.jsx so the store stays the leaf of the dependency graph. */
const VIEW_IDS = new Set([
  "chat",
  "library",
  "agents",
  "computers",
  "routines",
  "memory",
  "integrations",
  "activity",
]);

/**
 * Every tab the settings sheet has, so reopening lands where it was left.
 *
 * A stored tab that is not in this set is treated as never having been chosen
 * and falls back to General - which is right for a tab that was removed, and
 * silently wrong for one that was added and forgotten here. Memory, Agent Group
 * and Hooks were all added to the sheet without being added here, so closing
 * the sheet on any of the three and reopening it landed on General.
 *
 * It must stay in step with `GROUPS` in `features/settings/SettingsDialog`.
 */
const SETTINGS_TABS = new Set([
  "general",
  "identity",
  "appearance",
  "models",
  "voice",
  "computers",
  "memory",
  "group",
  "permissions",
  "hooks",
  "shortcuts",
  "workspace",
  "secrets",
  "about",
]);

function readSettingsTab() {
  const stored = readPref(PREF.settingsTab, "general");
  return SETTINGS_TABS.has(stored) ? stored : "general";
}

/**
 * The app's state, and the only thing any screen talks to.
 *
 * Everything below the store is unaware of where the data came from, which is
 * what let the workspace folder arrive without touching a single view:
 * `useWorkspacePersistence` reads the folder into this state on open and
 * mirrors every change back, so `sendMessage` stays a `setMessages` call and
 * the file it produces is somebody else's problem.
 *
 * The demo data is now a seed rather than the source. A brand new workspace is
 * written with it once so the app is never an empty shell; after that the
 * folder is the truth and this module's imports are only the fallback for a
 * session with no workspace at all.
 */
/**
 * Below this there is nothing a summary would save.
 *
 * Counted in provider entries rather than in messages, because one message
 * with a dozen tool calls in it is a dozen entries and is exactly the sort of
 * thing worth compacting.
 */
const MIN_TO_COMPACT = 8;

/**
 * How full the window has to be before the app compacts without being asked.
 *
 * Above the meter's amber line and below the loop's own trigger, so the
 * visible, once-per-conversation compaction happens before the invisible,
 * every-single-step one does.
 */
const AUTO_COMPACT_AT = 0.85;

export function AppProvider({ children }) {
  // ── navigation ──────────────────────────────────────────────────────────
  // Where you were is a preference, not app state: the app reopens on the
  // screen you left it on. The validators matter - a view or a machine that no
  // longer exists must fall back rather than render an empty shell.
  const [view, setView] = usePersistentState(PREF.view, "chat", (v) => VIEW_IDS.has(v));
  const [railExpanded, setRailExpanded] = usePersistentState(
    PREF.railExpanded,
    true,
    (v) => typeof v === "boolean"
  );
  // The chat's right panel. Whether it is open is a preference - it survives a
  // relaunch the way the rail does - but it is only ever mounted next to a live
  // thread, so nothing here has to know which thread: closing the last chat
  // hides the panel without discarding the choice to have it open.
  const [chatPanelOpen, setChatPanelOpen] = usePersistentState(
    PREF.chatPanelOpen,
    false,
    (v) => typeof v === "boolean"
  );
  const [chatPanelSections, setChatPanelSections] = usePersistentState(
    PREF.chatPanelSections,
    {},
    (v) => v !== null && typeof v === "object" && !Array.isArray(v)
  );
  /**
   * The conversation you were last in, remembered across a relaunch.
   *
   * Plain state until now, so every restart landed on "No conversation open" -
   * which also stranded the draft this app had just started keeping: the text
   * was safely on disk under a thread id nothing would open again. Restoring
   * the thread is what makes an unfinished message actually come back.
   *
   * Any string will do as a validator, for the same reason the active computer
   * takes one: which threads exist is not knowable until the workspace has been
   * read, and checking against the seed list here would throw away the id of the
   * conversation the user was in.
   */
  const [activeThreadId, setActiveThreadId] = usePersistentState(
    PREF.activeThreadId,
    null,
    (v) => v === null || typeof v === "string"
  );
  const [activeAgentId, setActiveAgentId] = useState(null);

  // Read once, at the first render, and never written here: the effect below
  // keeps it, and the profile overrules it the moment it arrives.
  const [mirroredAgentId] = useState(() => readPref(PREF.defaultAgentId, null));
  const [activeComputerId, setActiveComputerId] = usePersistentState(
    PREF.activeComputerId,
    null,
    // Any string will do: which computers exist is not knowable until the main
    // process has been asked, and validating against an empty seed list here
    // would throw away the id of the machine the user was last looking at.
    (v) => v === null || typeof v === "string"
  );
  const [activeRoutineId, setActiveRoutineId] = useState(null);

  // ── overlays ────────────────────────────────────────────────────────────
  // `open` is never persisted - relaunching into a modal would be a bug - but
  // the tab you were reading is.
  const [settings, setSettings] = useState(() => ({
    open: false,
    tab: readSettingsTab(),
  }));
  const [commandOpen, setCommandOpen] = useState(false);

  // ── data ────────────────────────────────────────────────────────────────
  const [agents, setAgents] = useState([]);
  const [threads, setThreads] = useState([]);
  const [messages, setMessages] = useState({});
  const [routines, setRoutines] = useState([]);
  const [memories, setMemories] = useState([]);
  // Machines are not seeded and are not mirrored from here. The backend owns
  // them, because it is the only thing that can see whether the container
  // behind a record is still running.
  const [computers, setComputers] = useState([]);
  const [activity, setActivity] = useState([]);
  // The permission document, in exactly the shape the backend reads back
  // out of `settings.permissions`: one workspace ruleset every agent inherits,
  // plus the overrides an agent wrote for itself. Both halves are flat arrays of
  // `{ tool, pattern, action }` because that is what the engine evaluates - a
  // second, prettier shape here would only be a translation layer to get wrong.
  const [permissions, setPermissions] = useState(() => ({
    workspace: defaultRules(),
    agents: {},
  }));
  const [user, setUser] = useState(CURRENT_USER);

  // Live agent turns, by thread, so a stop button can find the one it means -
  // and, since a turn can be spoken to as well as stopped, so can a message
  // typed while one is running. Each entry is `{ stop, steer }`.
  // A ref rather than state: nothing renders differently because a turn exists,
  // and a re-render per token would be the one thing this must not cause.
  const turns = useRef(new Map());

  // The latest `sendMessage`, for the one path that has to call it after an
  // await: a steer that misses its turn falls back to starting a new one, and
  // by then the closure that started the steer is looking at a transcript
  // missing the reply that finished in the meantime.
  const sendLater = useRef(null);

  // And the same trick for the compactor, which the turn-end handler reaches
  // for and which is defined a long way below it.
  const compactLater = useRef(null);

  // Conversations and history are not React state that happens to be saved -
  // they are the folder's, read from it on open and mirrored back on change.
  // The store above stays unchanged: nothing that calls `sendMessage` has to
  // know a file was written, which is the whole point of putting this here
  // rather than threading a save through every action.
  const { toast } = useToast();

  /**
   * Something appeared in the folder that this window did not put there.
   *
   * Almost always an agent making one: "create a coding agent" ends with a file
   * in `agents/`, and before the folder was watched the only sign of success
   * was a relaunch. Worth a word rather than a silent row appearing in a list
   * nobody is looking at.
   *
   * Named per collection, because "3 records appeared" is not something anyone
   * would say out loud.
   */
  const onArrival = useCallback(
    ({ collection, rows }) => {
      if (!rows?.length) return;
      const said = {
        agents: (row) => `${row.name ?? "A new agent"} has joined the team`,
        routines: (row) => `New routine: ${row.name ?? "untitled"}`,
        memory: (row) => `New memory: ${row.title ?? row.name ?? "untitled"}`,
      }[collection];
      if (!said) return;

      for (const row of rows.slice(0, 3)) toast({ title: said(row) });
      if (rows.length > 3) toast({ title: `...and ${rows.length - 3} more` });
    },
    [toast]
  );

  const { hydrated } = useWorkspacePersistence(
    {
      threads,
      setThreads,
      messages,
      setMessages,
      activity,
      setActivity,
      agents,
      setAgents,
      routines,
      setRoutines,
      computers,
      setComputers,
      memories,
      setMemories,
      permissions,
      setPermissions,
      user,
      setUser,
    },
    { onArrival }
  );

  /**
   * Whatever the folder holds is what exists, so a selection made before it was
   * read - or one that pointed at a record the user has since deleted from disk
   * - has to fall back rather than leave a screen addressing nothing.
   *
   * Not until the folder has actually been read, though. `threads` starts as the
   * seed fixtures, which never contain the id of the conversation the user was
   * last in, so running this on the first render would reset the remembered
   * thread to a demo one and write that over it - and the draft waiting in that
   * conversation would be stranded under an id nothing would open again.
   */
  useEffect(() => {
    if (!hydrated) return;
    // `activeThreadId &&` matters: null is not a dangling id, it is the
    // deliberate "no conversation open" state that the new-chat screen is
    // drawn from. Without the guard this effect snapped straight back to the
    // newest thread, so deleting the open conversation dropped the person into
    // an unrelated one instead of the empty screen.
    if (activeThreadId && threads.length && !threads.some((t) => t.id === activeThreadId)) {
      setActiveThreadId(threads[0].id);
    } else if (!threads.length && activeThreadId) {
      setActiveThreadId(null);
    }
  }, [hydrated, threads, activeThreadId, setActiveThreadId]);

  useEffect(() => {
    if (agents.length && !agents.some((a) => a.id === activeAgentId))
      setActiveAgentId(agents[0].id);
  }, [agents, activeAgentId]);

  useEffect(() => {
    if (computers.length && !computers.some((c) => c.id === activeComputerId)) {
      setActiveComputerId(computers[0].id);
    }
  }, [computers, activeComputerId, setActiveComputerId]);

  const logActivity = useCallback((event) => {
    setActivity((prev) => [
      {
        id: uid("act"),
        at: new Date().toISOString(),
        actor: "user",
        severity: "info",
        ...event,
      },
      ...prev,
    ]);
  }, []);

  // ── threads & messages ──────────────────────────────────────────────────
  /** Drop a thread's message list. For drafts being abandoned; see below. */
  const forgetMessages = useCallback((threadId) => {
    setMessages((prev) => {
      if (!(threadId in prev)) return prev;
      const next = { ...prev };
      delete next[threadId];
      return next;
    });
  }, []);

  /**
   * Temporary means temporary.
   *
   * A temporary conversation exists only while the person is in it. Leaving
   * it - opening another chat, starting a new one, going to another screen -
   * throws it away: the turn it was running is stopped, what the backend
   * held for it is forgotten, and it is gone from the state, which is the
   * only place it ever was. There is no way back to it, by design; a
   * conversation that can be recovered was never temporary.
   *
   * `except` is the one being opened, when that is itself temporary.
   */
  const discardTemporary = useCallback(
    (except = null) => {
      setThreads((prev) => {
        const going = prev.filter((t) => t.temporary && t.id !== except);
        if (!going.length) return prev;
        for (const t of going) {
          turns.current.get(t.id)?.stop();
          turns.current.delete(t.id);
          agentTools.forget(t.id)?.catch?.(() => {});
          forgetConversation(t.id)?.catch?.(() => {});
          forgetMessages(t.id);
        }
        const gone = new Set(going.map((t) => t.id));
        const next = prev.filter((t) => !gone.has(t.id));
        setActiveThreadId((cur) =>
          gone.has(cur) ? (next.find((t) => !t.draft)?.id ?? null) : cur
        );
        return next;
      });
    },
    [forgetMessages, setActiveThreadId]
  );

  const openThread = useCallback(
    (threadId) => {
      setActiveThreadId(threadId);
      setView("chat");
      discardTemporary(threadId);
      setThreads((prev) => {
        // Leaving a draft behind abandons it. An empty conversation the user
        // opened and walked away from is not a conversation, and keeping it
        // would fill the rail with rows nobody wrote anything in. Its message
        // list goes with it, or the mirror writes an empty file for it.
        for (const t of prev) if (t.draft && t.id !== threadId) forgetMessages(t.id);
        return prev
          .filter((t) => !t.draft || t.id === threadId)
          .map((t) => (t.id === threadId ? { ...t, unread: 0 } : t));
      });
    },
    [forgetMessages, discardTemporary]
  );

  // Going to another screen is leaving, too.
  useEffect(() => {
    if (view !== "chat") discardTemporary();
  }, [view, discardTemporary]);

  // Resolved against the agents the workspace actually has. A workspace with
  // no agents at all is allowed; the thread is simply addressed to nobody
  // until one exists.
  /**
   * Which teammate New chat starts with.
   *
   * One answer, in one place, because New chat is in four: the rail, the
   * shortcut, the command palette and the button on an empty transcript. Three
   * read the setting; the fourth started a thread with whichever agent the
   * person had last looked at, which on a fresh window is just the first one in
   * the folder. A workspace whose agents begin with Astrophysicist opened on
   * Astrophysicist however plainly the setting said otherwise.
   *
   * The mirror is the other half of it. The real setting lives in the profile
   * inside the workspace folder, which is read over IPC well after the first
   * render - so the very first New chat of a session, before that read lands,
   * had no setting to obey. The pane's copy in local storage is read
   * synchronously and is only ever a stand-in until the profile arrives.
   */
  const preferredAgentId = user?.preferences?.defaultAgentId ?? mirroredAgentId;
  const startingAgent = useMemo(
    () => defaultAgent(agents, preferredAgentId),
    [agents, preferredAgentId]
  );
  const startingAgentId = startingAgent?.id ?? null;

  useEffect(() => {
    const chosen = user?.preferences?.defaultAgentId;
    if (typeof chosen === "string" && chosen) writePref(PREF.defaultAgentId, chosen);
  }, [user?.preferences?.defaultAgentId]);

  const createThread = useCallback(
    (agentId) => {
      const id = uid("thr");
      // A caller that names an agent gets that agent - the button on an agent's
      // own page means the agent it is on. A caller that names nobody gets the
      // one the setting chose, which is the whole of what the setting is for.
      const agent = agents.find((b) => b.id === agentId) ?? startingAgent;
      // Validated on the way in: these come off the identity file, which a
      // person can edit, and an unknown word here would be a mode the loop
      // has no tools for.
      const wantedMode = user?.preferences?.defaultMode;
      const wantedApproval = user?.preferences?.defaultApproval;
      const thread = {
        id,
        agentId: agent?.id ?? null,
        title: "New conversation",
        mode: isMode(wantedMode) ? wantedMode : DEFAULT_MODE,
        approval: isApproval(wantedApproval) ? wantedApproval : DEFAULT_APPROVAL,
        draft: true,
        pinned: false,
        unread: 0,
        updatedAt: new Date().toISOString(),
        preview: "",
        messageCount: 0,
        computerAttached: agent?.computerId ?? null,
      };
      // Drafts are threads that have not earned their place yet: they render as
      // a live chat, but nothing lists them and nothing writes them to the folder
      // until a message is actually sent. Clicking New chat and changing your
      // mind should leave no trace.
      discardTemporary();
      setThreads((prev) => {
        // The draft being abandoned takes its message list with it. Left
        // behind, that list belongs to a thread the state no longer has, so
        // nothing marks it as a draft's and the mirror writes an empty
        // messages file for a conversation that never existed.
        for (const stale of prev) if (stale.draft) forgetMessages(stale.id);
        return [thread, ...prev.filter((t) => !t.draft)];
      });
      setMessages((prev) => ({ ...prev, [id]: [] }));
      setActiveThreadId(id);
      setView("chat");
      return id;
    },
    [
      agents,
      startingAgent,
      user?.preferences?.defaultMode,
      user?.preferences?.defaultApproval,
      forgetMessages,
      discardTemporary,
    ]
  );

  const chat = useChatRuntime();
  const chatModels = useMemo(() => allModels(chat.providers), [chat.providers]);

  /**
   * What each model costs, per million tokens.
   *
   * Two sources, merged. A price set on the model itself - entered where the
   * model is added - is the ordinary way in; the `prices` map in
   * `settings/models.json` is the explicit override and wins, which is how a
   * negotiated rate beats the number typed beside the model. Either way it
   * reaches `priceFor` keyed by model id, so the session panel can show a cost
   * instead of "No price".
   */
  const modelPrices = useMemo(() => {
    const merged = {};
    for (const model of chatModels) {
      if (Number.isFinite(model.input) && Number.isFinite(model.output)) {
        merged[model.id] = {
          input: model.input,
          output: model.output,
          // Only when set. Absent means "use the usual ratio", which is a
          // different statement from "cached reads are free".
          ...(Number.isFinite(model.cachedInput) ? { cachedInput: model.cachedInput } : {}),
          ...(Number.isFinite(model.cacheWrite) ? { cacheWrite: model.cacheWrite } : {}),
        };
      }
    }
    return { ...merged, ...(chat.settings.prices ?? {}) };
  }, [chatModels, chat.settings.prices]);

  /**
   * What an agent has actually done.
   *
   * The numbers on an agent's page used to be written once, when it was
   * created, and never again - so every agent read "0 messages, 0 tokens"
   * forever no matter how much work it did, and the one that shipped with
   * figures had been given them by hand. A statistic nothing updates is
   * decoration.
   *
   * Counted here rather than in the backend because this is where a turn is
   * known to belong to an agent: the backend knows a session ran and what it
   * cost, but the mapping from a turn to the teammate whose page shows it lives
   * with the roster. `lastActiveAt` moves for the same reason - an agent that
   * answered a second ago should not say "active 3h ago".
   */
  const recordActivity = useCallback((agentId, delta = {}) => {
    if (!agentId) return;
    setAgents((prev) =>
      prev.map((agent) => {
        if (agent.id !== agentId) return agent;
        const stats = agent.stats ?? {};
        return {
          ...agent,
          lastActiveAt: new Date().toISOString(),
          stats: {
            ...stats,
            messages: (stats.messages ?? 0) + (delta.messages ?? 0),
            routinesRun: (stats.routinesRun ?? 0) + (delta.routinesRun ?? 0),
            // Usage arrives as a running total for the turn, not an increment,
            // so the turn's own last figure is what gets added - once, on done.
            tokensUsed: (stats.tokensUsed ?? 0) + (delta.tokensUsed ?? 0),
          },
        };
      })
    );
  }, []);

  // The current threads and messages, for the callbacks that run after an
  // await and would otherwise be looking at the conversation as it was when
  // they started.
  const latestThreads = useRef(threads);
  latestThreads.current = threads;
  const latestMessages = useRef(messages);
  latestMessages.current = messages;
  const latestAgents = useRef(agents);
  latestAgents.current = agents;

  /** Threads with a naming request in flight, so two cannot race for one name. */
  const naming = useRef(new Set());

  /**
   * How to give the floor to the next agent in a group conversation.
   *
   * A ref rather than a value because the two halves need each other: folding a
   * turn's events is what discovers that somebody else is due to speak, and
   * starting that turn needs the fold to hand its events to.
   */
  const speakNext = useRef(null);

  /**
   * Give a conversation a name of its own.
   *
   * A thread used to be called the first 48 characters of the first thing typed
   * into it, which makes a sidebar of half-sentences that all begin the same
   * way. This asks the model for three to five words about what the
   * conversation is, once when it starts and again whenever it is compacted -
   * by which point the first message is often no longer what the work is about,
   * and the truncated question in the list is describing something that
   * happened an hour ago.
   *
   * Everything here fails quietly. The name it would replace is a usable name,
   * so a provider that is down, a model that answers with a paragraph, or a
   * thread the user has renamed themselves all end the same way: nothing
   * happens.
   */
  const nameThread = useCallback(
    async (threadId, { text } = {}) => {
      const thread = latestThreads.current.find((t) => t.id === threadId);
      // A name the user typed is theirs. A routine's thread is named after the
      // routine, which is more use than anything a model would write for it.
      if (!thread || thread.titleLocked || thread.routineId) return;
      if (naming.current.has(threadId)) return;

      // The caller may hand over the material - the message that has just been
      // typed has not reached state yet, and reading it back from there would
      // race the render that puts it there.
      const source = String(text ?? "").trim() || titleSource(latestMessages.current[threadId]);
      if (!source) return;

      naming.current.add(threadId);
      try {
        const agent = agents.find((a) => a.id === thread.agentId);
        const title = await chat.nameConversation({ agent, text: source });
        if (!title) return;
        setThreads((prev) =>
          prev.map((t) => (t.id === threadId && !t.titleLocked ? { ...t, title } : t))
        );
      } catch {
        /* the thread keeps the name it has */
      } finally {
        naming.current.delete(threadId);
      }
    },
    [agents, chat]
  );

  /**
   * One turn's events, folded into one message.
   *
   * Shared by the turn that starts here and the turn that was already running
   * when the window came back, which is the point: a rejoined transcript is
   * built by the same code as a live one, so there is no second rendering path
   * to keep in step with this one.
   */
  // Read here rather than inside the fold so the dependency is a boolean and
  // a turn's handler is not rebuilt every time anything else about the profile
  // changes.
  const autoCompactOn = user?.preferences?.autoCompact !== false;

  const foldTurn = useCallback(
    (threadId, messageId, agentId) => {
      // Who the room said should speak after this turn, and whether they have
      // been started. Held until the terminal event rather than acted on when
      // the room says so, because the room says so BEFORE `done` - and a turn
      // started then reads a transcript this one has not finished writing,
      // and gets its handle deleted by this one's `done` a moment later.
      let nextSpeaker = null;
      let chained = false;
      // How full the window was on this turn's last request. Kept here rather
      // than read back off the thread at `done`, because the thread in this
      // closure is the one from the render that started the turn and its gauge
      // is a turn out of date - which is exactly the turn that matters.
      let gauge = null;
      return (event) => {
        if (event.type === "started") return;
        // How full the window is, from the provider's own count of the last
        // request. On the thread, because it is a fact about the conversation
        // rather than about one message, and the header draws it from there.
        // The agent entered or left a worktree. The conversation follows it, so
        // the next turn starts where this one ended and the chip says so.
        if (
          event.type === "tool-end" &&
          event.ok !== false &&
          event.metadata &&
          "worktree" in event.metadata
        ) {
          const worktree = event.metadata.worktree ?? null;
          setThreads((prev) => prev.map((t) => (t.id === threadId ? { ...t, worktree } : t)));
        }
        if (event.type === "usage" && event.context?.window) {
          gauge = { used: event.context.used ?? 0, window: event.context.window, at: Date.now() };
          const settled = gauge;
          setThreads((prev) =>
            prev.map((t) => (t.id === threadId ? { ...t, context: settled } : t))
          );
        }
        // Only this turn's own handle. The next agent in a group may already
        // hold the thread's slot, and deleting that one made the reload path
        // attach a second listener to it - every chunk of its reply twice.
        if (event.type === "done" || event.type === "error") {
          const held = turns.current.get(threadId);
          if (held && (!held.messageId || held.messageId === messageId))
            turns.current.delete(threadId);
        }
        if (event.type === "done") {
          recordActivity(agentId, {
            messages: 1,
            tokensUsed: totalTokens(event.usage),
          });
          /**
           * The window is nearly full, so summarise once instead of a hundred
           * times.
           *
           * Between turns, never during one: a compaction that lands while the
           * agent is mid-reply changes the transcript under a turn that has
           * already built its own history from it. And only when nothing else
           * has taken the floor - in a room the next agent is already speaking,
           * and its turn is the one that would be cut off at the knees.
           */
          if (autoCompactOn && gauge?.window && gauge.used / gauge.window >= AUTO_COMPACT_AT) {
            if (!turns.current.has(threadId))
              compactLater.current?.(threadId, "", { automatic: true });
          }
        }
        // What failed reaches the activity feed, which is the audit trail the
        // app promises and for a long time did not keep: tool calls and refusals
        // from real turns never arrived here, so the feed showed what the app
        // did and nothing of what the agent did. Failures only - every read and
        // grep would drown the feed - and a refusal is drawn as blocked, because
        // that is what it is.
        if (event.type === "tool-end" && event.ok === false) {
          const denied = event.metadata?.error === "denied";
          logActivity({
            actor: "agent",
            agentId,
            category: denied ? "permission" : "failure",
            severity: denied ? "danger" : "warning",
            title: denied ? `${event.name} was refused` : `${event.name} failed`,
            detail:
              String(event.output ?? "")
                .split("\n")
                .find((line) => line.trim()) ?? "",
            target: threadId,
            tool: event.name,
          });
        }
        if (event.type === "error") {
          logActivity({
            actor: "agent",
            agentId,
            category: "failure",
            severity: "danger",
            title: "Turn ended in an error",
            detail: String(event.message ?? "").split("\n")[0],
            target: threadId,
          });
        }
        /**
         * The room said something about itself.
         *
         * Two of these arrive per group turn. The first names the speaker, and
         * has to arrive before the first token: the bubble was drawn against
         * whichever agent this window guessed, and in a room that guess is
         * routinely wrong. The second comes after the turn has ended and says
         * whether anybody is due to speak next.
         *
         * The chain is driven from here rather than from the backend
         * because everything the person needs in order to stop it - the
         * transcript, the bubbles, the stop button - is over here. A chain the
         * window cannot see is a chain the person cannot interrupt.
         */
        if (event.type === "group") {
          if (event.room) {
            setThreads((prev) =>
              prev.map((t) => {
                if (t.id !== threadId) return t;
                const next = { ...t, room: event.room };
                /*
                 * A handover moves who the conversation BELONGS to, which is
                 * the difference the person can see: the name and face at the
                 * top of the thread, and who what they type next goes to.
                 *
                 * Followed here rather than left to the room alone, because
                 * the thread's agent is what this window sends back as
                 * `primaryAgentId` on the next turn - so a thread that did not
                 * follow would hand the conversation straight back to the
                 * agent that had just given it away.
                 *
                 * `mode` follows too: a one-to-one chat that has just been
                 * handed over is a room now, and the chain that carries the
                 * new agent's first turn is the group one.
                 */
                const primary = event.room.primary;
                if (primary && primary !== t.agentId) {
                  next.agentId = primary;
                  next.mode = "group";
                }
                return next;
              })
            );
          }
          if (event.speaker) {
            setMessages((prev) => {
              const list = prev[threadId];
              if (!list) return prev;
              return {
                ...prev,
                [threadId]: list.map((m) =>
                  m.id === messageId
                    ? {
                        ...m,
                        agentId: event.speaker,
                        // The seat's own model. This window resolved the
                        // primary agent's before the turn started, because it
                        // is the only one it could know; the room seats
                        // whoever is due, and each of them has their own.
                        ...(event.modelRef ? { model: event.modelRef } : {}),
                      }
                    : m
                ),
              };
            });
          } else {
            nextSpeaker = event.room?.active ?? null;
          }
          return;
        }

        const terminal = event.type === "done" || event.type === "error";
        const grouped =
          terminal && latestThreads.current.find((t) => t.id === threadId)?.mode === "group";

        setMessages((prev) => {
          const list = prev[threadId];
          if (!list) return prev;
          let next = list.map((m) => {
            if (m.id !== messageId) return m;
            let after = applyEvent(m, event);
            // In a group, a reply that turned into a script for the other
            // agents is cut where the script starts, and a reply that is just
            // "pass" is the agent declining its turn.
            if (grouped) {
              const others = latestAgents.current
                .filter((one) => one.id !== agentId)
                .map((one) => one.name);
              after = { ...after, parts: cutScriptParts(after.parts, others) };
            }
            return {
              ...after,
              // `content` stays the plain text so every existing view - the
              // preview line, the copy button, the markdown renderer - keeps
              // working without knowing parts exist.
              content: textOf(after),
              status:
                event.type === "error"
                  ? "error"
                  : event.type === "done"
                    ? textOf(after).trim() || after.parts?.some((p) => p.type === "tool")
                      ? "sent"
                      : "error"
                    : "streaming",
              error: event.type === "error" ? event.message : m.error,
              usage: event.usage ?? m.usage,
            };
          });
          // A turn that said nothing: "pass", or a model that thought and then
          // produced no words. It is MARKED, not removed, and the difference is
          // a real bug's worth of difference. A message taken out of this list
          // is still on disk, where it was written mid-stream and left saying
          // `streaming`; the next load read that copy back, found a turn nobody
          // was running, and settled it as "This reply was interrupted and
          // never finished" - about a reply that had finished perfectly well
          // and simply had nothing to add. The ghost outlived its own turn.
          if (grouped && event.type === "done") {
            next = next.map((m) => {
              if (m.id !== messageId) return m;
              const said = m.content.trim();
              const spoke = m.parts?.some((p) => p.type === "tool") || (said && !isPass(said));
              return spoke ? m : { ...m, status: "sent", quiet: true };
            });
          }
          // The next agent reads the transcript as it is now - cut, or with the
          // quiet turn marked - rather than as it was a few events ago. Started
          // from inside the update so it is the same list; guarded because an
          // updater may run twice.
          if (terminal && nextSpeaker && !chained) {
            chained = true;
            const speaker = nextSpeaker;
            queueMicrotask(() => speakNext.current?.(threadId, speaker, next));
          }
          return { ...prev, [threadId]: next };
        });
      };
    },
    [recordActivity, nameThread, logActivity, autoCompactOn]
  );

  /**
   * Who wrote each reply, for a transcript several agents share.
   *
   * Only used in a group. Everywhere else there is one agent and naming it in
   * front of every line would be noise the model then imitates.
   */
  const speakerLabel = useCallback(
    (message) => {
      if (message.role !== "agent" || !message.agentId) return null;
      return agents.find((one) => one.id === message.agentId)?.name ?? null;
    },
    [agents]
  );

  /**
   * Give the floor to the next agent in a group conversation.
   *
   * The same thing `sendMessage` does at the end, minus the part about a
   * person: no message is added, nothing is steered, and the thread's preview
   * and title are left alone, because nobody said anything - the room simply
   * moved on. `continuation` tells the backend this is the next link in a
   * chain rather than an answer to something just said, so the floor is not
   * reset underneath it.
   */
  const takeTheFloor = useCallback(
    (threadId, agentId, spoken = null) => {
      const thread = latestThreads.current.find((t) => t.id === threadId);
      const agent = agents.find((one) => one.id === agentId) ?? null;
      // A room can name an agent that has since been paused or deleted. The
      // chain stops rather than erroring: the floor is back with the person,
      // which is where a conversation with nobody to speak into it belongs.
      if (!thread || !agent || isPaused(agent)) return;

      // The transcript the last turn left, handed over directly: the state
      // the ref mirrors may not have caught up with that turn's last event.
      const list = spoken ?? latestMessages.current[threadId] ?? [];
      const replyId = uid("msg");
      const model = chat.resolveModel(agent);

      setMessages((prev) => ({
        ...prev,
        [threadId]: [
          ...(prev[threadId] ?? []),
          {
            id: replyId,
            role: "agent",
            agentId: agent.id,
            content: "",
            createdAt: new Date().toISOString(),
            status: "streaming",
            model: model?.ref ?? null,
          },
        ],
      }));

      const handle = runTurn({
        threadId,
        agentId: agent.id,
        modelRef: model?.ref,
        messageId: replyId,
        mode: "group",
        approval: isApproval(thread.approval) ? thread.approval : DEFAULT_APPROVAL,
        cwd: thread?.worktree?.path ?? undefined,
        worktree: thread?.worktree ?? undefined,
        history: toHistory(list, { label: speakerLabel }),
        primaryAgentId: thread.agentId,
        continuation: true,
        roster: thread.room?.roster ?? undefined,
        on: foldTurn(threadId, replyId, agent.id),
      });
      turns.current.set(threadId, Object.assign(handle, { messageId: replyId }));
    },
    [agents, chat, foldTurn, speakerLabel]
  );
  speakNext.current = takeTheFloor;

  /**
   * Try that answer again.
   *
   * What Retry used to do was find the last thing the person had typed and
   * send it a second time. In a one-to-one chat that looked almost right and
   * still left a duplicate of your own message in the transcript. In a group it
   * was worse than wrong: sending a message resets the floor, so retrying one
   * agent's failed reply put all eight back in the queue and replayed the
   * entire debate - the reply that failed being the one thing it did not
   * retry.
   *
   * So it is the answer that is retried, not the question. The failed reply is
   * removed, the transcript goes back to the moment before it was written, and
   * the same agent is asked again from there. Nothing the person said is
   * repeated, because they only said it once.
   */
  const retryMessage = useCallback(
    (threadId, messageId) => {
      const thread = latestThreads.current.find((t) => t.id === threadId);
      const list = latestMessages.current[threadId] ?? [];
      const index = list.findIndex((m) => m.id === messageId);
      if (!thread || index === -1) return false;

      // Retrying a question means retrying the answer to it: the message
      // itself stays, everything after it goes.
      const target = list[index];
      const from = target.role === "user" ? index + 1 : index;
      const before = list.slice(0, from);

      // Who is being asked again. The reply carries its own agent; a question
      // is answered by whoever answered it, or by whoever the thread belongs to.
      const wanted =
        target.role === "user"
          ? (list.slice(from).find((m) => m.role === "agent" && m.agentId)?.agentId ??
            thread.agentId)
          : (target.agentId ?? thread.agentId);
      const agent = agents.find((one) => one.id === wanted) ?? null;
      if (!agent) return false;
      if (isPaused(agent)) {
        toast({ variant: "warning", title: pausedReason(agent) });
        return false;
      }

      // A turn already running on this thread is the one being retried, or is
      // about to collide with the retry. Either way it stops first.
      const live = turns.current.get(threadId);
      if (live) {
        live.stop();
        turns.current.delete(threadId);
      }

      const mode = isMode(thread.mode) ? thread.mode : DEFAULT_MODE;
      const group = mode === "group";
      const replyId = uid("msg");
      const model = chat.resolveModel(agent);

      setMessages((prev) => ({
        ...prev,
        [threadId]: [
          ...before,
          {
            id: replyId,
            role: "agent",
            agentId: agent.id,
            content: "",
            createdAt: new Date().toISOString(),
            status: "streaming",
            model: model?.ref ?? null,
          },
        ],
      }));

      const handle = runTurn({
        threadId,
        agentId: agent.id,
        modelRef: model?.ref,
        messageId: replyId,
        mode,
        approval: isApproval(thread.approval) ? thread.approval : DEFAULT_APPROVAL,
        cwd: thread?.worktree?.path ?? undefined,
        worktree: thread?.worktree ?? undefined,
        history: toHistory(before, { label: group ? speakerLabel : null }),
        // The room is told who is speaking rather than asked: it has already
        // moved past the turn that failed.
        ...(group
          ? {
              primaryAgentId: thread.agentId,
              speaker: agent.id,
              roster: thread.room?.roster ?? undefined,
            }
          : {}),
        on: foldTurn(threadId, replyId, agent.id),
      });
      turns.current.set(threadId, Object.assign(handle, { messageId: replyId }));
      return true;
    },
    [agents, chat, foldTurn, speakerLabel, toast]
  );

  /**
   * The events a rejoined turn should replay, with the reply made ready for
   * them.
   *
   * The log is the whole turn from its first event, and the message the
   * window left behind already holds most of it - the renderer saves on a
   * debounce, so a reload mid-turn finds the reply written up to a moment
   * ago. Replaying the log onto that reply wrote every part a second time:
   * the thought, the text, and forty tool cards with the same call ids,
   * every one of them then settled as "stopped before it finished". So the
   * reply is emptied first and the log rebuilds it, which is what "the same
   * events through the same fold" was always supposed to mean.
   *
   * A log that has shed its head cannot rebuild anything, so that reply is
   * kept as saved and only the live stream is attached.
   */
  const rejoinEvents = useCallback((turn) => {
    const events = turn.events ?? [];
    if (turn.truncated || !events.length) return [];
    setMessages((prev) => {
      const list = prev[turn.threadId];
      if (!list?.some((m) => m.id === turn.messageId)) return prev;
      return {
        ...prev,
        [turn.threadId]: list.map((m) =>
          m.id === turn.messageId ? { ...m, parts: [], content: "", status: "streaming" } : m
        ),
      };
    });
    return events;
  }, []);

  /**
   * Rejoin whatever was still running.
   *
   * The turn outlives the window by design - the loop, the tools and the key
   * are all in the backend - so a reload in the middle of one used to leave a
   * message spinning forever over work that was still happening and still
   * writing files. The backend keeps the event log; this asks for it once, on
   * open, and plays it into the message the window left behind.
   *
   * A turn whose message is gone (the thread was deleted, the folder was
   * emptied) is left alone rather than cancelled: it may be a subagent's, and
   * killing work because the window forgot about it is the wrong instinct.
   */
  useEffect(() => {
    // After the folder has been read, not before: both halves of this need the
    // messages to exist. A turn is rejoined by replaying its log into the reply
    // the window left behind, and a reply left spinning by a crash can only be
    // recognised once it has been loaded.
    if (!hydrated || !isAgentAvailable()) return undefined;
    let alive = true;
    const detach = [];

    activeTurns().then((live) => {
      if (!alive) return;
      for (const turn of live) {
        if (!turn.threadId || !turn.messageId) continue;
        if (turns.current.has(turn.threadId)) continue;
        const events = rejoinEvents(turn);
        const handle = attachTurn({
          id: turn.id,
          events,
          on: foldTurn(turn.threadId, turn.messageId, turn.agentId),
        });
        turns.current.set(turn.threadId, Object.assign(handle, { messageId: turn.messageId }));
        detach.push(handle.stop);
      }

      /*
       * And whatever is left spinning with nothing behind it.
       *
       * A reload rejoins its turn, above. A crash, a force quit, or a machine
       * that lost power does not: the turn died with the process, but the
       * reply was written to disk mid-stream and comes back saying
       * "streaming". Nothing ever settled it, so the bubble span forever and
       * the composer offered Stop for a turn that had not existed since the
       * last time the app was open.
       *
       * Settled the way stopping one is settled - what it managed to say is
       * kept, and `stopped` marks that it did not get to finish - because
       * that is exactly what happened to it.
       */
      const stillLive = new Set(live.map((turn) => turn.messageId).filter(Boolean));
      setMessages((prev) => {
        let touched = false;
        const next = {};
        for (const [id, list] of Object.entries(prev)) {
          next[id] = list.map((m) => {
            if (m.status !== "streaming" || stillLive.has(m.id)) return m;
            touched = true;
            return { ...m, status: m.content?.trim() ? "sent" : "error", stopped: true };
          });
        }
        return touched ? next : prev;
      });
    });

    return () => {
      alive = false;
    };
  }, [hydrated, foldTurn, rejoinEvents]);

  /**
   * Send a message, and stream the reply back into the transcript.
   *
   * Everything about talking to a provider lives in `chat-runtime`; this owns
   * what a message is and nothing else. The two halves meet at three callbacks,
   * which is what keeps the store free of any idea what a chat completion looks
   * like.
   *
   * Sending into a thread that is already working steers that turn instead of
   * starting a second one. Two turns in one conversation would answer the same
   * question twice from two transcripts and race each other's tool calls, and
   * the thing the person actually meant - "not that, do this" - is only worth
   * anything if it reaches the turn that is doing the wrong thing.
   */
  const sendMessage = useCallback(
    (threadId, content, options = {}) => {
      const text = String(content ?? "").trim();
      const attachments = options.attachments ?? [];
      // A message that is only a picture is a real message; "look at this" is
      // implied by the act of attaching it.
      if (!text && !attachments.length) return;

      // Text only. Attachments cross the bridge as part of a message the loop
      // builds from history, and a turn that is mid-flight has already built
      // its own - so the composer keeps files back for the next message rather
      // than this one silently dropping them.
      const live = turns.current.get(threadId);
      if (text && !attachments.length && live?.steer) {
        live.steer(text).then((landed) => {
          // The turn finished while the message was crossing. Nobody did
          // anything wrong and the words still exist, so they start a turn of
          // their own - which is what would have happened a moment later.
          if (!landed) sendLater.current?.(threadId, content, options);
        });
        return;
      }

      const at = new Date().toISOString();
      const message = {
        id: uid("msg"),
        role: "user",
        content: text,
        createdAt: at,
        status: "sent",
        ...(attachments.length ? { attachments } : {}),
      };

      const thread = threads.find((t) => t.id === threadId);
      const agent = agents.find((a) => a.id === thread?.agentId);
      // The mode this message was sent in. The picker writes it onto the thread,
      // so a reload or a routine reading the record later sees the same answer
      // the composer was showing when the person pressed send.
      const mode = isMode(options.mode) ? options.mode : (thread?.mode ?? DEFAULT_MODE);
      // The approval dial, read the same way: from the thread, where the
      // composer wrote it. Never from a default that loosens.
      const approval = isApproval(thread?.approval) ? thread.approval : DEFAULT_APPROVAL;
      const history = [...(messages[threadId] ?? []), message];
      // The first thing said in a conversation is what the conversation is
      // about, so this is where it gets its name.
      const firstMessage = !thread?.messageCount;

      setMessages((prev) => ({ ...prev, [threadId]: [...(prev[threadId] ?? []), message] }));
      setThreads((prev) =>
        prev.map((t) =>
          t.id === threadId
            ? {
                ...t,
                // The first message is what makes a draft a conversation.
                draft: false,
                mode,
                updatedAt: at,
                preview: (text || attachments[0]?.name || "").slice(0, 120),
                messageCount: (t.messageCount ?? 0) + 1,
                title: t.messageCount
                  ? t.title
                  : (text || attachments[0]?.name || "Untitled").slice(0, 48),
              }
            : t
        )
      );

      const replyId = uid("msg");
      const model =
        (options.model && chatModels.find((m) => m.ref === options.model)) ||
        chat.resolveModel(agent);
      setMessages((prev) => ({
        ...prev,
        [threadId]: [
          ...(prev[threadId] ?? []),
          {
            id: replyId,
            role: "agent",
            agentId: agent?.id,
            content: "",
            createdAt: new Date().toISOString(),
            status: "streaming",
            model: model?.ref ?? null,
          },
        ],
      }));

      // Fired here rather than after the reply, because it is the question
      // that names the thread and waiting for an answer would leave the list
      // showing a truncated sentence for as long as the agent takes to think.
      // Not awaited: a title is not part of sending a message.
      if (firstMessage) nameThread(threadId, { text });

      const patchReply = (patch) =>
        setMessages((prev) => {
          const list = prev[threadId];
          if (!list) return prev;
          return {
            ...prev,
            [threadId]: list.map((m) => (m.id === replyId ? { ...m, ...patch(m) } : m)),
          };
        });

      // A paused agent answers nothing.
      //
      // The backend refuses this too, and that refusal is the one that
      // counts - it is the only one a reloaded window or a scheduled routine
      // cannot get past. This one exists so the person sees the reason in the
      // reply where they are looking, immediately, and because the browser
      // preview has no backend to refuse anything at all. The message the two
      // produce is the same sentence, written once on each side.
      if (isPaused(agent)) {
        patchReply(() => ({ status: "error", error: pausedReason(agent) }));
        return;
      }

      // The desktop app runs the agent loop, which can call tools. A browser
      // preview has no backend and no tools, so it falls back to a plain
      // stream - the same conversation, without the hands.
      if (isAgentAvailable()) {
        // In a group, `@` in the message is an instruction about who answers,
        // and it outranks whatever the agents had planned between themselves.
        // Resolved here because this is where the agent list is; the main
        // process is handed ids, not names.
        const group = mode === "group";
        const mentioned = group ? mentionedIds(text, mentionSpecs({ agents })).agent : undefined;

        const handle = runTurn({
          threadId,
          agentId: agent?.id,
          modelRef: options.model ?? model?.ref,
          messageId: replyId,
          mode,
          approval,
          primaryAgentId: group ? thread?.agentId : undefined,
          mentioned,
          roster: group ? (thread?.room?.roster ?? undefined) : undefined,
          // The worktree this conversation entered, if any. Sent every turn
          // because main keeps no per-thread state of its own.
          cwd: thread?.worktree?.path ?? undefined,
          worktree: thread?.worktree ?? undefined,
          history: toHistory(history, { label: mode === "group" ? speakerLabel : null }),
          on: foldTurn(threadId, replyId, agent?.id),
        });
        turns.current.set(threadId, Object.assign(handle, { messageId: replyId }));
        return;
      }

      chat.send({
        threadId,
        agent,
        modelRef: options.model,
        history,
        systemPrompt: agent?.systemPrompt,
        onDelta: (delta) => patchReply((m) => ({ content: m.content + delta })),
        onDone: (event) =>
          patchReply((m) => ({
            // A reply that produced nothing is a failure the user can see, not
            // an empty bubble they have to guess at.
            status: m.content.trim() ? "sent" : "error",
            error: m.content.trim() ? undefined : "The model returned nothing.",
            usage: event?.usage ?? undefined,
            finish: event?.finish ?? undefined,
          })),
        onError: (error) =>
          patchReply(() => ({
            status: "error",
            // Partial text is kept. Half an answer plus the reason it stopped
            // is more use than a bubble that erases what did arrive.
            error: error?.message ?? "The response could not be completed.",
          })),
      });
    },
    [threads, agents, messages, chat, chatModels, foldTurn, nameThread, speakerLabel]
  );

  sendLater.current = sendMessage;

  /**
   * A word in the transcript that nobody said.
   *
   * `/context` answering with a number, `/autocompact` confirming what it just
   * changed - these belong where the person is looking, which is the
   * conversation, and they are not part of the conversation. So they are
   * `system` messages: drawn as a centred grey line, and skipped entirely by
   * `toHistory`, so no model is ever asked to read the app talking to itself.
   */
  const noteThread = useCallback((threadId, content, extra = {}) => {
    if (!threadId || !String(content ?? "").trim()) return null;
    const note = {
      id: uid("msg"),
      role: "system",
      content: String(content),
      createdAt: new Date().toISOString(),
      ...extra,
    };
    setMessages((prev) => ({ ...prev, [threadId]: [...(prev[threadId] ?? []), note] }));
    return note;
  }, []);

  /**
   * Draw a line across the conversation and carry a note over it.
   *
   * The only compaction there is. The agent loop does not do this - a comment
   * here used to say it did, at seventy percent and without asking, and that
   * sentence described a design that was never built. Two callers, then:
   * `/compact` for the moment a person can see the work turning a
   * corner, and the automatic trigger below for the moment the window is
   * nearly full and the alternative is the loop paying for a summary on every
   * single step from here on.
   *
   * Nothing is deleted. The note is appended as a message, and `toHistory`
   * treats it as the new beginning - so the model gets the summary and what
   * came after, while the person can still scroll back through all of it.
   * That is the whole reason the boundary is a message rather than a field on
   * the thread: it is visible, it survives a reload with the transcript it
   * belongs to, and undoing it is deleting one message.
   */
  const compactThread = useCallback(
    async (threadId, focus = "", { automatic = false } = {}) => {
      const list = messages[threadId] ?? [];
      const history = toHistory(list);
      // A summary of two messages costs a round trip and saves nothing.
      if (history.length < MIN_TO_COMPACT) {
        if (!automatic) {
          noteThread(threadId, "There is not enough conversation yet to be worth summarising.");
        }
        return false;
      }

      const thread = threads.find((t) => t.id === threadId);
      const agent = agents.find((a) => a.id === thread?.agentId);
      const model = chat.resolveModel(agent);
      const pending = noteThread(threadId, "Summarising the conversation so far…");

      try {
        const { summary } = await summarize({
          modelRef: model?.ref,
          history,
          focus: String(focus ?? ""),
        });
        const kept = list.length;
        setMessages((prev) => {
          const current = prev[threadId];
          if (!current) return prev;
          return {
            ...prev,
            [threadId]: current.map((m) =>
              m.id === pending?.id
                ? {
                    ...m,
                    content: automatic
                      ? `The window was nearly full, so the first ${kept} messages were summarised. Everything above is still here to read; the agent now reads the note instead.`
                      : `The first ${kept} messages were summarised. Everything above is still here to read; the agent now reads the note instead.`,
                    summary,
                  }
                : m
            ),
          };
        });
        // The name was taken from the first message, and that message is now
        // on the far side of the line just drawn - so the title describes
        // something the agent can no longer see. Written again from what the
        // conversation has become.
        //
        // This used to be a branch on a `warning` event with `kind: "context"`,
        // which nothing has ever emitted: compaction is decided here, in the
        // window, and the backend has no opinion about it. So the rename never
        // happened once.
        nameThread(threadId);
        return true;
      } catch (error) {
        setMessages((prev) => {
          const current = prev[threadId];
          if (!current) return prev;
          return {
            ...prev,
            [threadId]: current.map((m) =>
              m.id === pending?.id
                ? {
                    ...m,
                    content:
                      `The conversation could not be summarised. ${error?.message ?? ""}`.trim(),
                  }
                : m
            ),
          };
        });
        return false;
      }
    },
    [messages, threads, agents, chat, noteThread, nameThread]
  );

  compactLater.current = compactThread;

  /** Stop a reply that is still arriving. */
  const stopMessage = useCallback(
    (threadId) => {
      const turn = turns.current.get(threadId);
      if (turn) {
        turn.stop();
        turns.current.delete(threadId);
      } else if (!chat.cancel(threadId)) {
        return;
      }
      setMessages((prev) => {
        const list = prev[threadId];
        if (!list) return prev;
        return {
          ...prev,
          [threadId]: list.map((m) =>
            m.status === "streaming"
              ? { ...m, status: m.content.trim() ? "sent" : "error", stopped: true }
              : m
          ),
        };
      });
    },
    [chat]
  );

  const togglePinThread = useCallback((threadId) => {
    setThreads((prev) => prev.map((t) => (t.id === threadId ? { ...t, pinned: !t.pinned } : t)));
  }, []);

  /**
   * The user's own name for a conversation.
   *
   * `titleLocked` is the whole point: a thread somebody named must never be
   * renamed by the automatic namer, which would otherwise overwrite it on the
   * next compaction and look like the app arguing.
   */
  const renameThread = useCallback((threadId, title) => {
    setThreads((prev) =>
      prev.map((t) => (t.id === threadId ? { ...t, title, titleLocked: true } : t))
    );
  }, []);

  /**
   * Which mode this conversation runs in.
   *
   * On the thread rather than in the composer, because it decides what the
   * agent may do - and a control whose value is lost when you switch chats and
   * comes back as something else is a control nobody can trust. It also means
   * the approval card can switch a conversation to Autonomous itself.
   */
  const setThreadMode = useCallback((threadId, next) => {
    if (!isMode(next)) return;
    setThreads((prev) => prev.map((t) => (t.id === threadId ? { ...t, mode: next } : t)));
  }, []);

  /**
   * The approval dial, on the thread for the same reason the mode is: it
   * decides what happens without the user looking, and a control that came
   * back different after switching chats would be one they could not trust.
   */
  /** Leave a worktree from the chip: the folder stays, the conversation stops using it. */
  const setThreadWorktree = useCallback((threadId, next) => {
    setThreads((prev) =>
      prev.map((t) => (t.id === threadId ? { ...t, worktree: next ?? null } : t))
    );
  }, []);

  const setThreadApproval = useCallback((threadId, next) => {
    if (!isApproval(next)) return;
    setThreads((prev) => prev.map((t) => (t.id === threadId ? { ...t, approval: next } : t)));
  }, []);

  /**
   * Whether this conversation is written down.
   *
   * A flag on the thread rather than state in the composer, because the only
   * thing that can honour it is the mirror, and the mirror reads threads. It
   * was a dashed border and nothing else for as long as it lived in the
   * picker: the border said the conversation would not be saved and every
   * message in it was written to the workspace folder anyway.
   */
  const setThreadTemporary = useCallback((threadId, next) => {
    setThreads((prev) =>
      prev.map((t) => (t.id === threadId ? { ...t, temporary: Boolean(next) } : t))
    );
  }, []);

  const deleteThread = useCallback(
    (threadId) => {
      // A conversation ending drops what was scoped to it: the permissions the
      // user granted with "always", the task list, the record of which files were
      // read. None of that should survive into a conversation that has nothing to
      // do with it, least of all the permissions.
      agentTools.forget(threadId)?.catch?.(() => {});
      // And the runs it started, which are still in memory in the backend
      // with nowhere left to be shown.
      forgetConversation(threadId)?.catch?.(() => {});

      setThreads((prev) => {
        const next = prev.filter((t) => t.id !== threadId);
        // Back to the empty screen, not into whatever conversation happens to be
        // newest. Landing in an unrelated thread after a delete reads as though
        // the wrong thing was deleted, and the next message goes somewhere the
        // person did not choose.
        setActiveThreadId((cur) => (cur === threadId ? null : cur));
        return next;
      });
      setMessages((prev) => {
        const next = { ...prev };
        delete next[threadId];
        return next;
      });
      // `setActiveThreadId` comes from `usePersistentState`, which hands back
      // `useState`'s own setter - stable, but the linter cannot see through the
      // custom hook to know that. Listed rather than silenced.
    },
    [setActiveThreadId]
  );

  /**
   * Take back the last thing that happened in a conversation.
   *
   * The menu item existed and raised "editing history is not available in this
   * preview build", which is a promise the app was not keeping. The real
   * behaviour is the useful one: an exchange is a question and its answer, so
   * removing "the last message" and leaving the question that produced it
   * behind would leave a thread that reads as if the agent was asked something
   * and said nothing.
   *
   * A turn still running is stopped first - deleting the message a stream is
   * writing into would leave the stream writing into nothing.
   */
  const deleteLastExchange = useCallback((threadId) => {
    turns.current.get(threadId)?.stop();
    turns.current.delete(threadId);
    setMessages((prev) => {
      const list = prev[threadId];
      if (!list?.length) return prev;
      // Back off the trailing agent replies, then the user turn beneath them.
      let cut = list.length;
      while (cut > 0 && list[cut - 1].role === "agent") cut -= 1;
      if (cut > 0 && list[cut - 1].role === "user") cut -= 1;
      // A thread whose last block is neither still loses one, so the item is
      // never a no-op the person cannot explain.
      if (cut === list.length) cut -= 1;
      return { ...prev, [threadId]: list.slice(0, Math.max(0, cut)) };
    });
    setThreads((prev) =>
      prev.map((t) =>
        t.id === threadId
          ? {
              ...t,
              messageCount: Math.max(0, (t.messageCount ?? 1) - 1),
              updatedAt: new Date().toISOString(),
            }
          : t
      )
    );
  }, []);

  // ── routines ────────────────────────────────────────────────────────────
  const toggleRoutine = useCallback(
    (routineId) => {
      setRoutines((prev) =>
        prev.map((r) => {
          if (r.id !== routineId) return r;
          logActivity({
            agentId: r.agentId,
            category: "routine",
            title: (r.enabled ? "Disabled" : "Enabled") + " routine",
            detail: r.name,
            target: r.id,
          });
          return { ...r, enabled: !r.enabled };
        })
      );
    },
    [logActivity]
  );

  const runRoutine = useCallback(
    (routineId) => {
      const routine = routines.find((r) => r.id === routineId);
      setRoutines((prev) =>
        prev.map((r) =>
          r.id === routineId
            ? {
                ...r,
                lastRun: {
                  at: new Date().toISOString(),
                  status: "running",
                  durationMs: 0,
                  summary: "Started manually.",
                },
              }
            : r
        )
      );
      logActivity({
        agentId: routine?.agentId,
        category: "routine",
        title: "Routine run started",
        detail: routine?.name,
        target: routineId,
      });
      // The scheduler owns the run and writes the record; this only reflects
      // what came back so the row updates without waiting for the folder
      // watcher. What was here before was a `setTimeout` that wrote
      // "Completed with no findings" two and a half seconds later - a fixture
      // wearing a feature's clothes, and the reason a routine set to run every
      // weekday at nine did nothing at nine, or ever.
      if (!isSchedulerAvailable()) {
        toast({
          title: "Routines run in the desktop app",
          description: "This preview has no scheduler behind it.",
        });
        return;
      }

      scheduler
        .run(routineId)
        .then((run) => {
          setRoutines((prev) =>
            prev.map((r) =>
              r.id === routineId
                ? { ...r, lastRun: run, runHistory: [run, ...(r.runHistory ?? [])].slice(0, 20) }
                : r
            )
          );
          logActivity({
            agentId: routine?.agentId,
            category: "routine",
            title: run?.status === "error" ? "Routine failed" : "Routine finished",
            detail: run?.summary ?? routine?.name,
            target: routineId,
          });
          recordActivity(routine?.agentId, { routinesRun: 1 });
        })
        .catch((error) => {
          // A rejection here is a refusal to start, not a run that failed: no
          // workspace, the routine is gone, it is already going, or its agent
          // is paused. The scheduler writes a record for every run that
          // actually happened, so the optimistic "running" row is put back the
          // way it was rather than turned into a failure nobody performed.
          setRoutines((prev) =>
            prev.map((r) => (r.id === routineId ? { ...r, lastRun: routine?.lastRun ?? null } : r))
          );
          toast({
            variant: "danger",
            title: "Routine not started",
            description: error?.message ?? String(error),
          });
        });
    },
    [routines, logActivity, recordActivity, toast]
  );

  /**
   * Runs nobody in this window asked for.
   *
   * The scheduler keeps time in the backend whether a window is open or
   * not, so most runs start without anything here calling `runRoutine`. The
   * folder watcher would eventually redraw the row, but "eventually" is not
   * what someone watching a routine fire at nine o'clock wants, and the
   * activity log and the agent's own counters would have missed it entirely.
   *
   * So the same two things happen as for a manual run, from the same events
   * the scheduler already emits.
   */
  const latestRoutines = useRef(routines);
  latestRoutines.current = routines;

  /**
   * Watch a routine's run as it happens.
   *
   * The scheduler writes the run's thread and its two messages before the
   * turn starts and says so in the event, so this has everything the chat
   * screen needs: the thread goes into the list, the messages under it, and
   * the turn is attached exactly the way a reload rejoins one - the record's
   * events so far replayed, then the live stream. Somebody who opens the
   * conversation sees the tool calls land as the routine makes them, which is
   * the difference between "Running" and knowing what it is doing.
   */
  const watchRoutineRun = useCallback(
    async (event) => {
      const { turnId, threadId, messageId, agentId, thread, messages: opened } = event;
      if (!turnId || !threadId || !messageId) return;
      if (turns.current.has(threadId)) return;

      if (thread) {
        setThreads((prev) =>
          prev.some((t) => t.id === threadId)
            ? prev.map((t) => (t.id === threadId ? { ...t, ...thread } : t))
            : [thread, ...prev]
        );
      }
      setMessages((prev) => {
        const list = prev[threadId] ?? [];
        const known = new Set(list.map((m) => m.id));
        const fresh = (opened ?? []).filter((m) => !known.has(m.id));
        return fresh.length ? { ...prev, [threadId]: [...list, ...fresh] } : prev;
      });

      const record = await turnRecord(turnId);
      if (turns.current.has(threadId)) return;
      const handle = attachTurn({
        id: turnId,
        events: rejoinEvents({ ...record, threadId, messageId }),
        on: foldTurn(threadId, messageId, agentId),
      });
      turns.current.set(threadId, Object.assign(handle, { messageId }));
    },
    [foldTurn, rejoinEvents]
  );

  useEffect(() => {
    if (!isSchedulerAvailable()) return undefined;
    return scheduler.onEvent((event) => {
      if (event?.type === "routine:started") {
        setRoutines((prev) =>
          prev.map((r) =>
            r.id === event.routineId
              ? {
                  ...r,
                  lastRun: {
                    at: event.at,
                    status: "running",
                    durationMs: 0,
                    summary: "Started on schedule.",
                  },
                }
              : r
          )
        );
        watchRoutineRun(event);
        return;
      }
      if (event?.type !== "routine:finished") return;

      const run = event.run;
      // Read through the ref, not the closed-over list: this effect subscribes
      // once and would otherwise be looking at the roster as it was when it
      // did, which for a routine created since is no roster at all.
      const routine = latestRoutines.current.find((r) => r.id === event.routineId);
      setRoutines((prev) =>
        prev.map((r) =>
          r.id === event.routineId
            ? {
                ...r,
                lastRun: run,
                runHistory: [run, ...(r.runHistory ?? [])].slice(0, 20),
                // The run just moved the schedule on. Without carrying it the
                // row goes on pointing at the run that already happened -
                // "next run 2 hours ago" - and so does the Next up tile.
                schedule: event.schedule ?? r.schedule,
              }
            : r
        )
      );
      logActivity({
        agentId: routine?.agentId,
        category: "routine",
        title: run?.status === "error" ? "Routine failed" : "Routine finished",
        detail: run?.summary ?? routine?.name,
        target: event.routineId,
      });
      recordActivity(routine?.agentId, { routinesRun: 1 });
    });
  }, [logActivity, recordActivity, watchRoutineRun]);

  const saveRoutine = useCallback((routineId, patch) => {
    setRoutines((prev) => {
      const next = prev.map((r) => (r.id === routineId ? { ...r, ...patch } : r));

      // Recompute when the schedule moved, rather than waiting for the next
      // tick. The scheduler would fill this in within half a minute anyway, but
      // "Next run: -" sitting under a schedule someone has just set reads as a
      // schedule that did not take.
      const changed = next.find((r) => r.id === routineId);
      if (changed && ("schedule" in patch || "enabled" in patch) && isSchedulerAvailable()) {
        scheduler
          .next(changed)
          .then(({ at }) => {
            setRoutines((current) =>
              current.map((r) =>
                r.id === routineId && r.schedule?.nextRunAt !== at
                  ? { ...r, schedule: { ...r.schedule, nextRunAt: at } }
                  : r
              )
            );
          })
          .catch(() => {
            // The tick will get to it. A failure here is not worth a message.
          });
      }
      return next;
    });
  }, []);

  const createRoutine = useCallback(
    (draft = {}) => {
      const id = uid("rt");
      const routine = {
        icon: "Repeat",
        description: "",
        markdown: "",
        tags: [],
        agentId: agents[0]?.id ?? null,
        ...draft,
        id,
        name: draft.name || "Untitled routine",
        schedule: draft.schedule ?? {
          kind: "manual",
          expression: null,
          humanLabel: "Run manually",
          nextRunAt: null,
        },
        enabled: draft.enabled ?? true,
        lastRun: null,
        runHistory: [],
      };
      setRoutines((prev) => [...prev, routine]);
      setActiveRoutineId(id);
      logActivity({
        agentId: routine.agentId,
        category: "routine",
        severity: "success",
        title: "Routine created",
        detail: routine.name,
        target: id,
      });
      return id;
    },
    [agents, logActivity]
  );

  const deleteRoutine = useCallback(
    (routineId) => {
      const routine = routines.find((r) => r.id === routineId);
      setRoutines((prev) => prev.filter((r) => r.id !== routineId));
      setActiveRoutineId((cur) => (cur === routineId ? null : cur));
      logActivity({
        agentId: routine?.agentId,
        category: "routine",
        severity: "warning",
        title: "Routine deleted",
        detail: routine?.name,
        target: routineId,
      });
    },
    [routines, logActivity]
  );

  // ── memory ──────────────────────────────────────────────────────────────
  const pinMemory = useCallback((id) => {
    setMemories((prev) => prev.map((m) => (m.id === id ? { ...m, pinned: !m.pinned } : m)));
  }, []);

  const deleteMemory = useCallback((id) => {
    setMemories((prev) => prev.filter((m) => m.id !== id));
  }, []);

  const saveMemory = useCallback((id, patch) => {
    setMemories((prev) => prev.map((m) => (m.id === id ? { ...m, ...patch } : m)));
  }, []);

  const createMemory = useCallback(
    (draft = {}) => {
      const id = uid("mem");
      const at = new Date().toISOString();
      const memory = {
        kind: "fact",
        title: "Untitled memory",
        body: "",
        tags: [],
        agentId: agents[0]?.id ?? null,
        ...draft,
        id,
        // Written by a human, so it starts fully trusted and unused.
        source: "pinned",
        confidence: 1,
        useCount: 0,
        createdAt: at,
        lastUsedAt: at,
        pinned: Boolean(draft.pinned),
      };
      setMemories((prev) => [memory, ...prev]);
      logActivity({
        agentId: memory.agentId,
        category: "memory",
        severity: "success",
        title: "Memory added",
        detail: memory.title,
        target: id,
      });
      return id;
    },
    [agents, logActivity]
  );

  // ── agents ────────────────────────────────────────────────────────────────
  const saveAgent = useCallback((agentId, patch) => {
    setAgents((prev) => prev.map((b) => (b.id === agentId ? { ...b, ...patch } : b)));
  }, []);

  const createAgent = useCallback(
    (draft = {}) => {
      const id = uid("agent");
      const name = draft.name || "Untitled agent";
      const agent = {
        status: "idle",
        model: agents[0]?.model,
        computerId: null,
        description: "",
        systemPrompt: "",
        tags: [],
        ...draft,
        id,
        name,
        handle: draft.handle || "@" + name.toLowerCase().replace(/\s+/g, ""),
        role: draft.role || "New teammate",
        createdAt: new Date().toISOString(),
        lastActiveAt: new Date().toISOString(),
        stats: { messages: 0, routinesRun: 0, tokensUsed: 0 },
        routineIds: [],
        memoryCount: 0,
      };
      setAgents((prev) => [...prev, agent]);
      // No seeded overrides. A new agent inherits the workspace ruleset, which is
      // what "defaults" has to mean if changing them is ever to reach anyone.
      setActiveAgentId(id);
      return id;
    },
    [agents]
  );

  const deleteAgent = useCallback((agentId) => {
    setAgents((prev) => {
      const next = prev.filter((b) => b.id !== agentId);
      setActiveAgentId((cur) => (cur === agentId ? (next[0]?.id ?? null) : cur));
      return next;
    });
  }, []);

  /** Replace the workspace ruleset. Takes an array or an updater, like setState. */
  const setWorkspaceRules = useCallback((next) => {
    setPermissions((prev) => ({
      ...prev,
      workspace: typeof next === "function" ? next(prev.workspace ?? []) : next,
    }));
  }, []);

  /** Replace one agent's own overrides. An empty list is dropped rather than
   *  stored, so "this agent says nothing" reads the same on disk as it does in
   *  the UI and an agent never accumulates an empty array nobody can see. */
  const setAgentRules = useCallback((agentId, next) => {
    setPermissions((prev) => {
      const current = prev.agents?.[agentId] ?? [];
      const rules = typeof next === "function" ? next(current) : next;
      const agents = { ...(prev.agents ?? {}) };
      if (rules?.length) agents[agentId] = rules;
      else delete agents[agentId];
      return { ...prev, agents };
    });
  }, []);

  /**
   * Set one rule on one agent.
   *
   * The narrow entry point, for the places that answer a single prompt rather
   * than open the matrix - a blocked tool call offering "allow this".
   */
  const setPermission = useCallback(
    (agentId, tool, action, pattern = ANY) => {
      setAgentRules(agentId, (prev) => [
        ...prev.filter((r) => !(r.tool === tool && r.pattern === pattern)),
        { tool, pattern, action },
      ]);
      logActivity({
        agentId,
        category: "permission",
        severity: action === "deny" ? "warning" : "success",
        title: "Permission set to " + action,
        detail: pattern === ANY ? tool : tool + " " + pattern,
        target: tool,
      });
    },
    [logActivity, setAgentRules]
  );

  // ── computers ───────────────────────────────────────────────────────────
  /**
   * The machines, as the provider currently sees them.
   *
   * Asked rather than remembered. A container can be stopped from a terminal
   * and a cloud sandbox can be reaped for idleness, so a status held in
   * renderer state is right only until someone touches the machine from
   * somewhere else - which, for a feature whose whole point is that the machine
   * is real, is most of the time.
   */
  const reloadComputers = useCallback(async () => {
    if (!areComputersAvailable()) return [];
    try {
      const rows = await machines.list();
      setComputers(rows);
      return rows;
    } catch {
      // A provider that cannot be reached is not a reason to blank the screen;
      // the rows already on it are the last thing that was true.
      return [];
    }
  }, []);

  useEffect(() => {
    reloadComputers();
  }, [reloadComputers]);

  /**
   * The meters, kept moving.
   *
   * Only while the computers screen is open. Polling a Daytona API every ten
   * seconds on behalf of a screen nobody is looking at is somebody money.
   */
  useEffect(() => {
    if (view !== "computers" || !areComputersAvailable()) return undefined;
    const timer = window.setInterval(() => reloadComputers(), 10_000);
    return () => window.clearInterval(timer);
  }, [view, reloadComputers]);

  /** One lifecycle verb, run for real, with the row refreshed from the answer. */
  const runComputerAction = useCallback(
    async (computerId, verb) => {
      const computer = computers.find((c) => c.id === computerId);
      // Optimistic only as far as the spinner: the real status comes back from
      // the provider a moment later and overwrites whatever this guessed.
      setComputers((prev) =>
        prev.map((c) => (c.id === computerId ? { ...c, status: "provisioning" } : c))
      );
      try {
        const updated = await machines[verb](computerId);
        setComputers((prev) => prev.map((c) => (c.id === computerId ? updated : c)));
        logActivity({
          computerId,
          category: "computer",
          title: "Computer " + updated.status,
          detail: computer?.name,
          target: computerId,
        });
        return updated;
      } catch (error) {
        await reloadComputers();
        logActivity({
          computerId,
          category: "computer",
          severity: "error",
          title: "Computer " + verb + " failed",
          detail: error?.message ?? String(error),
          target: computerId,
        });
        throw error;
      }
    },
    [computers, logActivity, reloadComputers]
  );

  /**
   * Make a machine, for real.
   *
   * Awaited rather than optimistic. Provisioning a Docker computer for the
   * first time builds the sandbox image, which pulls Ubuntu and a compiler and
   * takes minutes - and a row that appeared instantly and settled into
   * "running" on a two-and-a-half second timer is the exact lie this pass
   * exists to remove.
   */
  const createComputer = useCallback(
    async (draft = {}) => {
      const made = await machines.create(draft);
      setComputers((prev) => [...prev.filter((c) => c.id !== made.id), made]);
      setActiveComputerId(made.id);
      logActivity({
        computerId: made.id,
        category: "computer",
        title: "Computer provisioned",
        detail: made.name,
        target: made.id,
      });
      return made.id;
    },
    [logActivity, setActiveComputerId]
  );

  const deleteComputer = useCallback(
    async (computerId) => {
      const computer = computers.find((c) => c.id === computerId);
      // The machine goes first. Dropping the row and then failing to destroy
      // the container would leave something running that nothing can reach.
      await machines.remove(computerId);
      setComputers((prev) => {
        const next = prev.filter((c) => c.id !== computerId);
        setActiveComputerId((cur) => (cur === computerId ? (next[0]?.id ?? null) : cur));
        return next;
      });
      logActivity({
        computerId,
        category: "computer",
        severity: "warning",
        title: "Computer destroyed",
        detail: computer?.name,
        target: computerId,
      });
    },
    [computers, logActivity, setActiveComputerId]
  );

  /**
   * Who may use a machine.
   *
   * Written through the backend because it touches two records - the
   * computer's roster and each agent's `computerId` - and two writers that can
   * disagree is how an agent ends up pointed at a machine that has never heard
   * of it. The agent rows are updated from the answer rather than guessed.
   */
  const assignComputer = useCallback(async (computerId, agentIds) => {
    const updated = await machines.assign(computerId, agentIds);
    setComputers((prev) => prev.map((c) => (c.id === computerId ? updated : c)));
    setAgents((prev) =>
      prev.map((agent) => {
        const isOn = (updated.assignedAgentIds ?? []).includes(agent.id);
        if (isOn) return agent.computerId === computerId ? agent : { ...agent, computerId };
        return agent.computerId === computerId ? { ...agent, computerId: null } : agent;
      })
    );
    return updated;
  }, []);

  // ── profile ─────────────────────────────────────────────────────────────
  /** Merge a partial profile onto the single user. Preferences are untouched. */
  const updateProfile = useCallback((patch) => {
    setUser((prev) => ({ ...prev, ...patch }));
  }, []);

  // ── preferences ─────────────────────────────────────────────────────────
  const setPreference = useCallback((key, value) => {
    setUser((prev) => ({ ...prev, preferences: { ...prev.preferences, [key]: value } }));
  }, []);

  /** Several at once, so a reset is one write and one render, not fifteen. */
  const setPreferences = useCallback((patch) => {
    setUser((prev) => ({ ...prev, preferences: { ...prev.preferences, ...patch } }));
  }, []);

  const resetPreferences = useCallback(() => {
    setUser((prev) => ({ ...prev, preferences: { ...PREFERENCE_DEFAULTS } }));
  }, []);

  const openSettings = useCallback((tab) => {
    setSettings((s) => ({ open: true, tab: tab ?? s.tab }));
    if (tab) writePref(PREF.settingsTab, tab);
  }, []);
  const closeSettings = useCallback(() => setSettings((s) => ({ ...s, open: false })), []);
  const setSettingsTab = useCallback((tab) => {
    setSettings((s) => ({ ...s, tab }));
    writePref(PREF.settingsTab, tab);
  }, []);
  const toggleRail = useCallback(() => setRailExpanded((v) => !v), []);
  const toggleChatPanel = useCallback(() => setChatPanelOpen((v) => !v), [setChatPanelOpen]);
  /** Collapse state per section id; absent means the section's own default. */
  const toggleChatPanelSection = useCallback(
    (id) => setChatPanelSections((prev) => ({ ...prev, [id]: !prev?.[id] })),
    [setChatPanelSections]
  );

  const value = useMemo(
    () => ({
      // no `now` here on purpose: a timestamp captured into a memoized context
      // value is frozen for the life of the provider, which is how the whole
      // app ended up reading its clock off a date from months ago. Anything
      // that needs the time reads it at the moment it formats.
      view,
      setView,
      railExpanded,
      toggleRail,
      setRailExpanded,
      chatPanelOpen,
      setChatPanelOpen,
      toggleChatPanel,
      chatPanelSections,
      toggleChatPanelSection,
      activeThreadId,
      setActiveThreadId,
      activeAgentId,
      startingAgentId,
      setActiveAgentId,
      activeComputerId,
      setActiveComputerId,
      activeRoutineId,
      setActiveRoutineId,
      settings,
      openSettings,
      closeSettings,
      setSettingsTab,
      commandOpen,
      setCommandOpen,
      agents,
      threads,
      messages,
      routines,
      memories,
      computers,
      activity,
      permissions,
      user,
      openThread,
      createThread,
      sendMessage,
      noteThread,
      compactThread,
      stopMessage,
      retryMessage,
      chatReady: chat.ready,
      // The models the user actually configured. Empty until a provider
      // exists, which is what every picker keys off to decide whether it is
      // showing a real choice or the demo catalogue.
      chatModels,
      defaultModelRef: chat.settings.defaultModel,
      modelPrices,
      togglePinThread,
      renameThread,
      setThreadMode,
      setThreadApproval,
      setThreadTemporary,
      setThreadWorktree,
      deleteThread,
      deleteLastExchange,
      toggleRoutine,
      runRoutine,
      saveRoutine,
      createRoutine,
      deleteRoutine,
      pinMemory,
      deleteMemory,
      saveMemory,
      createMemory,
      saveAgent,
      createAgent,
      deleteAgent,
      setPermission,
      setWorkspaceRules,
      setAgentRules,
      runComputerAction,
      reloadComputers,
      createComputer,
      deleteComputer,
      assignComputer,
      updateProfile,
      setPreference,
      setPreferences,
      resetPreferences,
      logActivity,
    }),
    [
      view,
      railExpanded,
      toggleRail,
      chatPanelOpen,
      setChatPanelOpen,
      toggleChatPanel,
      chatPanelSections,
      toggleChatPanelSection,
      activeThreadId,
      activeAgentId,
      startingAgentId,
      activeComputerId,
      activeRoutineId,
      settings,
      openSettings,
      closeSettings,
      setSettingsTab,
      commandOpen,
      agents,
      threads,
      messages,
      routines,
      memories,
      computers,
      activity,
      permissions,
      user,
      openThread,
      createThread,
      sendMessage,
      noteThread,
      compactThread,
      stopMessage,
      retryMessage,
      chat.ready,
      chatModels,
      chat.settings.defaultModel,
      modelPrices,
      togglePinThread,
      renameThread,
      setThreadMode,
      setThreadApproval,
      setThreadTemporary,
      setThreadWorktree,
      deleteThread,
      deleteLastExchange,
      toggleRoutine,
      runRoutine,
      saveRoutine,
      createRoutine,
      deleteRoutine,
      pinMemory,
      deleteMemory,
      saveMemory,
      createMemory,
      saveAgent,
      createAgent,
      deleteAgent,
      setPermission,
      setWorkspaceRules,
      setAgentRules,
      runComputerAction,
      reloadComputers,
      assignComputer,
      createComputer,
      deleteComputer,
      updateProfile,
      setPreference,
      logActivity,
    ]
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp() {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used inside <AppProvider>");
  return ctx;
}

/**
 * The same, or null outside the provider. For the few pieces of the message
 * renderer that are also drawn on the preview page, where there is no app to
 * navigate.
 */
export function useAppIfAny() {
  return useContext(AppContext);
}
