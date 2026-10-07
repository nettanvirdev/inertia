import * as React from "react";
import { useCallback, useMemo, useState } from "react";
import { MOTION } from "@/lib/motion";
import { Library as LibraryIcon } from "@/components/icons";
import { cn } from "@/lib/utils";
import { AppProvider, useApp } from "@/lib/store";
import { WorkspaceProvider, useWorkspace } from "@/lib/workspace";
import { useHotkeys } from "@/hooks/use-hotkey";
import { GLOBAL_SHORTCUTS } from "@/data/shortcuts";
import { useTheme } from "@/lib/theme";
import { applyAppearance } from "@/lib/appearance";
import { Titlebar } from "@/components/layout/Titlebar";
import { Rail, RailHeader, visibleNavItems } from "@/components/layout/Rail";
import { ViewBoundary } from "@/components/layout/ViewBoundary";
import { CommandPalette } from "@/components/ui/command-palette";
import { ChatView } from "@/features/chat/ChatView";
import { LibraryView } from "@/features/library/LibraryView";
import { AgentsView } from "@/features/agents/AgentsView";
import { ComputersView } from "@/features/computers/ComputersView";
import { RoutinesView } from "@/features/routines/RoutinesView";
import { MemoryView } from "@/features/memory/MemoryView";
import { IntegrationsView } from "@/features/integrations/IntegrationsView";
import { ActivityView } from "@/features/activity/ActivityView";
import { SettingsDialog } from "@/features/settings";
import { SetupView } from "@/features/onboarding";
import { Spinner } from "@/components/ui/spinner";
import { useNotifications } from "@/lib/notify";

const VIEWS = {
  chat: ChatView,
  library: LibraryView,
  agents: AgentsView,
  computers: ComputersView,
  routines: RoutinesView,
  memory: MemoryView,
  integrations: IntegrationsView,
  activity: ActivityView,
};

function Shell() {
  // Subscribed once, at the root. A team that finished has to be announced
  // whether the person is looking at the chat, the computers screen or a
  // settings dialog - which is exactly when it is worth announcing.
  useNotifications();

  const {
    view,
    setView,
    toggleRail,
    commandOpen,
    setCommandOpen,
    openSettings,
    agents,
    threads,
    routines,
    computers,
    openThread,
    createThread,
    startingAgentId,
    setActiveAgentId,
    setActiveComputerId,
    activeThreadId,
    togglePinThread,
    user,
  } = useApp();

  /**
   * The screens that exist, and getting off one that has just stopped existing.
   *
   * Switching a feature off while looking at its screen must not leave the
   * person staring at a view they can no longer navigate back to. The redirect
   * runs on the list rather than on a specific setting, so the next feature that
   * becomes optional inherits the behaviour without anyone remembering to add
   * it here.
   */
  const navItems = React.useMemo(() => visibleNavItems(user?.preferences), [user?.preferences]);
  React.useEffect(() => {
    if (view === "chat" || view === "library") return;
    if (navItems.some((item) => item.id === view)) return;
    setView("chat");
  }, [navItems, view, setView]);

  // Appearance is applied here rather than in each pane, so a choice takes
  // effect the moment it is saved and again on the next launch without the
  // settings sheet ever being opened.
  const { resolved } = useTheme();
  React.useEffect(() => {
    applyAppearance(user.preferences, resolved === "dark");
  }, [user.preferences, resolved]);

  // The chat list in the order the rail shows it, which is the order stepping
  // to the next or previous chat has to follow for the two to agree.
  const orderedThreads = useMemo(
    () =>
      threads
        .filter((t) => !t.draft && !t.temporary)
        .sort((a, b) => new Date(b.updatedAt) - new Date(a.updatedAt)),
    [threads]
  );

  const stepThread = useCallback(
    (delta) => {
      if (orderedThreads.length === 0) return;
      const at = orderedThreads.findIndex((t) => t.id === activeThreadId);
      // Wrapping, because a list you can walk off the end of makes the key feel
      // broken rather than finished.
      const next = (at + delta + orderedThreads.length) % orderedThreads.length;
      openThread(orderedThreads[at === -1 ? 0 : next].id);
    },
    [orderedThreads, activeThreadId, openThread]
  );

  // One handler per action name in the catalogue. Anything the catalogue lists
  // and this object does not answer is a broken row, which is the failure this
  // whole arrangement exists to make impossible.
  const actions = useMemo(
    () => ({
      "command-palette": () => setCommandOpen(true),
      "toggle-sidebar": toggleRail,
      settings: () => openSettings("general"),
      // The same agent every other New chat starts with; the store works it
      // out once so no two buttons can answer it differently.
      "new-thread": () => createThread(startingAgentId),
      "next-thread": () => stepThread(1),
      "prev-thread": () => stepThread(-1),
      "toggle-pin": () => {
        if (activeThreadId) togglePinThread(activeThreadId);
      },
    }),
    [
      setCommandOpen,
      toggleRail,
      openSettings,
      startingAgentId,
      createThread,
      stepThread,
      activeThreadId,
      togglePinThread,
    ]
  );

  const bindings = useMemo(() => {
    const map = {};
    for (const shortcut of GLOBAL_SHORTCUTS) {
      map[shortcut.combo] =
        shortcut.action === "view" ? () => setView(shortcut.view) : actions[shortcut.action];
    }
    return map;
  }, [actions, setView]);

  useHotkeys(bindings);

  const groups = useMemo(
    () => [
      {
        label: "Go to",
        items: [{ id: "library", label: "Library", icon: LibraryIcon }, ...navItems].map(
          (item) => ({
            id: `view:${item.id}`,
            label: item.label,
            icon: item.icon,
            keywords: ["navigate", "open", item.id],
          })
        ),
      },
      {
        label: "Chats",
        items: orderedThreads.slice(0, 8).map((t) => ({
          id: `thread:${t.id}`,
          label: t.title,
          hint: agents.find((b) => b.id === t.agentId)?.name,
          keywords: ["thread", "conversation"],
        })),
      },
      {
        label: "Agents",
        items: agents.map((b) => ({
          id: `agent:${b.id}`,
          label: b.name,
          hint: b.role,
          keywords: ["agent", "teammate", b.handle ?? ""],
        })),
      },
      {
        label: "Computers",
        items: computers.map((c) => ({
          id: `computer:${c.id}`,
          label: c.name,
          hint: c.provider,
          keywords: ["machine", "sandbox", "desktop"],
        })),
      },
      {
        label: "Routines",
        items: routines.slice(0, 8).map((r) => ({
          id: `routine:${r.id}`,
          label: r.name,
          hint: r.schedule?.humanLabel,
          keywords: ["routine", "schedule", "playbook"],
        })),
      },
      {
        label: "Settings",
        items: [
          { id: "settings:appearance", label: "Appearance", keywords: ["theme", "dark", "light"] },
          { id: "settings:models", label: "Models & providers", keywords: ["api", "key", "llm"] },
          { id: "settings:permissions", label: "Permissions", keywords: ["allow", "deny"] },
          { id: "settings:shortcuts", label: "Keyboard shortcuts", keywords: ["keys"] },
        ],
      },
    ],
    [agents, orderedThreads, routines, computers, navItems]
  );

  const onCommand = useCallback(
    (item) => {
      const [kind, id] = String(item.id).split(":");
      if (kind === "view") setView(id);
      if (kind === "thread") openThread(id);
      if (kind === "agent") {
        setActiveAgentId(id);
        setView("agents");
      }
      if (kind === "computer") {
        setActiveComputerId(id);
        setView("computers");
      }
      if (kind === "routine") setView("routines");
      if (kind === "settings") openSettings(id);
      setCommandOpen(false);
    },
    [setView, openThread, setActiveAgentId, setActiveComputerId, openSettings, setCommandOpen]
  );

  // A view whose feature is off is not merely hidden from the rail - it does
  // not render, so nothing that still holds a link to it can reach the screen.
  const available = view === "chat" || view === "library" || navItems.some((i) => i.id === view);
  const View = (available ? VIEWS[view] : null) ?? ChatView;
  const viewLabel = [{ id: "library", label: "Library" }, ...navItems].find(
    (i) => i.id === view
  )?.label;

  return (
    <>
      <div className="flex min-h-0 flex-1">
        <Rail />
        {/* Keyed on the view, so the boundary resets when the user navigates
            away - a screen that broke on one workspace's data must not keep
            the app in an error state after they have moved on from it - and
            so the incoming screen fades in rather than replacing the last one
            between two frames. The key is on the column itself, which costs
            one element and no extra layer of layout. */}
        <main
          key={view}
          className="flex min-h-0 min-w-0 flex-1 flex-col bg-background animate-fade-in"
        >
          <ViewBoundary name={view} label={viewLabel}>
            <View />
          </ViewBoundary>
        </main>
      </div>

      <CommandPalette
        open={commandOpen}
        onOpenChange={setCommandOpen}
        groups={groups}
        onSelect={onCommand}
        placeholder="Search agents, chats, routines and settings…"
      />
      <SettingsDialog />
    </>
  );
}

/**
 * The window itself, which outlives whatever is inside it.
 *
 * The chrome is deliberately outside the workspace switch: the rounded shell and
 * the titlebar carry the minimise, maximise and close controls, so a user who is
 * still choosing a folder - or staring at a workspace that has gone missing -
 * has to be able to move and close the window like any other. Only the content
 * below the strip swaps.
 */
function Window() {
  const { phase, status } = useWorkspace();
  const [windowState, setWindowState] = useState("normal");
  const isMaximized = windowState === "maximized" || windowState === "fullscreen";

  /**
   * The window's shape, published on the document so CSS can read it.
   *
   * Dialogs, sheets and the command palette all portal to the body, outside the
   * rounded shell below, so they cannot be told about this through props. They
   * need it: a full-bleed scrim that ignores the rounding paints the corners the
   * shell left empty and squares the window off for as long as it is open.
   */
  React.useEffect(() => {
    document.documentElement.dataset.window = isMaximized ? "maximized" : "normal";
  }, [isMaximized]);

  /**
   * A beat of settling when the window changes shape.
   *
   * Maximising is the operating system moving the frame; the content inside
   * simply reflows to the new size on one frame, which reads as a jolt. A
   * brief fade on the shell - opacity only, see `globals.css` for why not a
   * scale - lets the contents land in the new frame rather than snap. Timed
   * from `MOTION.slow` so the attribute leaves when the animation does, and
   * skipped on mount: the first state is where the window already is.
   */
  const [settling, setSettling] = useState(false);
  const previousState = React.useRef(windowState);
  React.useEffect(() => {
    if (previousState.current === windowState) return undefined;
    previousState.current = windowState;
    setSettling(true);
    const timer = setTimeout(() => setSettling(false), MOTION.slow);
    return () => clearTimeout(timer);
  }, [windowState]);

  // "ready" alone is not enough. The folder can be remembered and still be gone,
  // or no longer be a workspace, and in both cases the shell would be reading
  // from nothing - so those land back on setup with the reason stated.
  const healthy = phase === "ready" && !status?.missing && !status?.stale;

  return (
    <div
      data-settling={settling || undefined}
      className={cn(
        "flex flex-col overflow-hidden bg-background",
        // The corners ease between the two shapes. Only the corners: a radius
        // is a paint, while the hairline margin and the height are a relayout
        // of the entire window on every frame they move, for one pixel nobody
        // would see travel. Those snap, and the settle above covers the cut.
        "transition-[border-radius] duration-[var(--motion-base)] ease-[var(--ease-out)]",
        isMaximized ? "h-full" : "m-[1px] h-[calc(100%-2px)] rounded-window"
      )}
    >
      {/* The sidebar's own header lives in the titlebar strip, so the brand
          is stated once and the rail can start at the first real action.
          There is no rail during setup, so the strip carries controls only. */}
      <Titlebar
        leading={healthy ? <RailHeader /> : null}
        windowState={windowState}
        onWindowStateChange={setWindowState}
      />

      {phase === "loading" ? (
        <div className="flex min-h-0 flex-1 items-center justify-center bg-background">
          <Spinner size="lg" className="text-muted-foreground" label="Opening workspace" />
        </div>
      ) : healthy ? (
        <Shell />
      ) : (
        <SetupView />
      )}
    </div>
  );
}

export default function App() {
  // Workspace first: the app store reads from the folder, so the question of
  // whether there is a folder has to be answerable before anything else mounts.
  return (
    <WorkspaceProvider>
      <AppProvider>
        <Window />
      </AppProvider>
    </WorkspaceProvider>
  );
}
