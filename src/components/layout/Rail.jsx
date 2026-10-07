import { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  Agent,
  Blocks,
  Brain,
  Check,
  ChevronDown,
  ChevronsLeft,
  ChevronsRight,
  Command,
  User,
  Library,
  Monitor,
  Moon,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Repeat,
  Settings,
  Sun,
  SunMoon,
  Trash2,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { appearanceOf } from "@/lib/appearance";
import { useAvatarSrc } from "@/lib/avatar";
import { PREF, usePersistentToggle } from "@/lib/persist";
import { useTheme } from "@/lib/theme";
import { relativeTime } from "@/data";
import { Badge } from "@/components/ui/badge";
import { Tooltip } from "@/components/ui/tooltip";
import { Avatar } from "@/components/ui/avatar";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { Kbd, formatShortcut } from "@/components/ui/kbd";
import {
  DropdownMenu,
  MenuGroup,
  MenuItem,
  MenuLabel,
  MenuSeparator,
} from "@/components/ui/dropdown-menu";
import { visibleItems } from "@shared/features";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Collapse } from "@/components/ui/collapse";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { TitlebarButton } from "@/components/layout/Titlebar";

export const NAV_ITEMS = [
  { id: "agents", label: "Agents", icon: Agent },
  { id: "computers", label: "Computers", icon: Monitor },
  { id: "routines", label: "Routines", icon: Repeat },
  { id: "memory", label: "Memory", icon: Brain, needs: "memory" },
  { id: "integrations", label: "Integrations", icon: Blocks },
  { id: "activity", label: "Activity", icon: Activity },
];

/**
 * The screens that exist right now.
 *
 * A feature that is switched off should leave nothing behind - not a greyed
 * row, not a gap, and not a route that still answers if something links to it.
 * The predicate lives in `shared/features.js` so the rail, the palette and the
 * router all read one answer - the same one the backend reads from the
 * preference when it decides what a turn holds.
 */
export function visibleNavItems(preferences) {
  return visibleItems(NAV_ITEMS, preferences);
}

/**
 * The navigation rail. One column in both states - the 32px icon cell lands on
 * the same x whether expanded or collapsed, so switching never makes the icons
 * jump. Width and background animate together on the panel curve, and the
 * labels fade in beside the icons rather than appearing once the column has
 * finished opening.
 */
export function Rail() {
  const {
    view,
    setView,
    railExpanded,
    agents,
    threads,
    routines,
    activity,
    user,
    openThread,
    createThread,
    startingAgentId,
    togglePinThread,
    renameThread,
    deleteThread,
    activeThreadId,
    openSettings,
    setCommandOpen,
    setRailExpanded,
  } = useApp();

  // The screens that exist right now. A switched-off feature leaves no row.
  const navItems = useMemo(() => visibleNavItems(user?.preferences), [user?.preferences]);

  /**
   * "Sidebar on launch" beats the remembered rail state.
   *
   * The store restores `PREF.railExpanded`, which is where the rail was when
   * the window last closed. That is the right default until someone states a
   * preference, and then it is not: a person who asked for a collapsed rail did
   * not ask for "collapsed, unless you happened to leave it open".
   *
   * It runs on the VALUE rather than once on mount because preferences arrive
   * from the workspace a tick after the first render, so a mount-only effect
   * would apply the seed and then ignore the real answer. The ref is what keeps
   * a manual toggle afterwards: the same value never applies twice.
   */
  const sidebarDefault = appearanceOf(user?.preferences).sidebarDefault;
  const appliedDefault = useRef(null);
  useEffect(() => {
    if (appliedDefault.current === sidebarDefault) return;
    appliedDefault.current = sidebarDefault;
    setRailExpanded(sidebarDefault !== "collapsed");
  }, [sidebarDefault, setRailExpanded]);

  const counts = {
    routines: routines.filter((r) => r.lastRun?.status === "error").length,
    activity: activity.filter((e) => e.severity === "danger").length,
  };

  // Pinned is a complete list; Recent is a window onto the rest. Everything
  // else lives in the Library, which is where the full history belongs.
  // Neither drafts nor temporary conversations: the second is never listed
  // anywhere, because a conversation with a row to go back to is not temporary.
  const byRecency = [...threads.filter((t) => !t.draft && !t.temporary)].sort(
    (a, b) => new Date(b.updatedAt) - new Date(a.updatedAt),
  );
  const pinned = byRecency.filter((t) => t.pinned);
  const recent = byRecency.filter((t) => !t.pinned).slice(0, 7);

  // Whether a list is open is a preference, so it survives a relaunch - the
  // point of collapsing Recent is to keep it collapsed.
  const [pinnedOpen, , togglePinned] = usePersistentToggle(PREF.railPinnedOpen, true);
  const [recentOpen, , toggleRecent] = usePersistentToggle(PREF.railRecentOpen, true);

  /**
   * Which rows the person has already seen, so only a new one slides in.
   *
   * A conversation created or restored while they are looking should arrive;
   * the seven that were there when the window opened should simply be there.
   * The set is seeded from the first list that has anything in it rather than
   * from the first render, because the threads land a tick after the rail
   * mounts and a set seeded empty would greet every one of them as new.
   */
  const seen = useRef(new Set());
  if (seen.current.size === 0) for (const t of byRecency) seen.current.add(t.id);
  const fresh = (thread) => !seen.current.has(thread.id);
  useEffect(() => {
    for (const t of byRecency) seen.current.add(t.id);
  });

  return (
    <nav
      aria-label="Primary"
      style={{
        width: railExpanded ? "var(--rail-expanded)" : "var(--rail-collapsed)",
      }}
      className={cn(
        // pt-3, not pt-2: it drops the first row's centre onto 28px, which is
        // the centre of the 56px view header beside it - so New chat and the
        // agent name it sits next to share a baseline.
        "flex shrink-0 flex-col gap-1 overflow-hidden px-1.5 pb-2 pt-3",
        "transition-[width,background-color] duration-[var(--motion-panel)] ease-[var(--ease-out)]",
        railExpanded ? "bg-sidebar" : "bg-transparent hover:bg-sidebar",
      )}
    >
      <div className="flex flex-col gap-0.5">
        <RailRow
          expanded={railExpanded}
          icon={Plus}
          label="New chat"
          emphasis
          onClick={() => createThread(startingAgentId)}
        />
        <RailRow
          expanded={railExpanded}
          icon={Command}
          label="Search"
          trailing={<Kbd>{formatShortcut("mod+k")}</Kbd>}
          onClick={() => setCommandOpen(true)}
        />
        <RailRow
          expanded={railExpanded}
          icon={Library}
          label="Library"
          active={view === "library"}
          onClick={() => setView("library")}
        />
      </div>

      {/* The heading sits outside the rows' gap, so that when it folds to
          nothing the rows above and below it stay exactly where they were. */}
      <div className="mt-5 flex flex-col">
        <RailHeading expanded={railExpanded}>Workspace</RailHeading>
        <div className="flex flex-col gap-0.5">
          {navItems.map((item) => (
            <RailRow
              key={item.id}
              expanded={railExpanded}
              icon={item.icon}
              label={item.label}
              active={view === item.id}
              count={counts[item.id]}
              onClick={() => setView(item.id)}
            />
          ))}
        </div>
      </div>

      {railExpanded && (
        <div className="mt-5 flex min-h-0 flex-1 flex-col">
          <ScrollArea className="min-h-0 flex-1" fade>
            <div className="flex flex-col pt-0.5 pb-2">
              {pinned.length > 0 && (
                <RailSection
                  label="Pinned"
                  count={pinned.length}
                  open={pinnedOpen}
                  onToggle={togglePinned}
                >
                  {pinned.map((thread) => (
                    <ThreadRow
                      key={thread.id}
                      thread={thread}
                      fresh={fresh(thread)}
                      agent={agents.find((b) => b.id === thread.agentId)}
                      active={view === "chat" && activeThreadId === thread.id}
                      onOpen={openThread}
                      onTogglePin={togglePinThread}
                      onRename={renameThread}
                      onDelete={deleteThread}
                    />
                  ))}
                </RailSection>
              )}

              <RailSection
                label="Recent"
                count={recent.length}
                open={recentOpen}
                onToggle={toggleRecent}
                className={pinned.length > 0 ? "pt-3" : undefined}
              >
                {recent.map((thread) => (
                  <ThreadRow
                    key={thread.id}
                    thread={thread}
                    fresh={fresh(thread)}
                    agent={agents.find((b) => b.id === thread.agentId)}
                    active={view === "chat" && activeThreadId === thread.id}
                    onOpen={openThread}
                    onTogglePin={togglePinThread}
                    onRename={renameThread}
                    onDelete={deleteThread}
                  />
                ))}
              </RailSection>
            </div>
          </ScrollArea>
        </div>
      )}

      <div
        className={cn("mt-auto flex flex-col gap-0.5", railExpanded && "pt-2")}
      >
        <ThemeRow expanded={railExpanded} />
        <RailRow
          expanded={railExpanded}
          icon={Settings}
          label="Settings"
          onClick={() => openSettings("general")}
        />
        <UserRow
          expanded={railExpanded}
          user={user}
          onSettings={openSettings}
        />
      </div>
    </nav>
  );
}

/**
 * The rail's header, rendered by `App` into the titlebar strip rather than
 * inside the rail itself, so the app states its name once.
 *
 * It is titlebar furniture, so it is built to the titlebar's measurements and
 * not the sidebar's: a 14px glyph, an 11px label and a 44 x 32 control that IS
 * the same component as minimise/maximise/close. The left end of the strip and
 * the right end therefore read as one row of chrome rather than a brand block
 * sitting awkwardly beside a set of window buttons.
 *
 * Collapsed, the logo IS the expand control - hovering swaps the mark for the
 * chevron. At 44px there is room for exactly one target, and spending it on a
 * static logo would leave the only way back out of the collapsed rail hidden.
 */
export function RailHeader() {
  const { railExpanded, toggleRail } = useApp();

  return (
    <div
      style={{
        width: railExpanded ? "var(--rail-expanded)" : "var(--rail-collapsed)",
      }}
      className="flex h-8 shrink-0 items-center overflow-hidden transition-[width] duration-[var(--motion-panel)] ease-[var(--ease-out)]"
    >
      {railExpanded ? (
        <>
          {/* pl-3.5 puts the mark on 14px - the same left edge every row in the
              rail below starts from, so the column reads as one line. */}
          <span className="flex min-w-0 flex-1 items-center gap-2 pl-3.5">
            <img src="./assets/logo-64.png" alt="" className="size-3.5 shrink-0" />
            <span className="truncate text-[11px] font-medium text-foreground">
              Inertia
            </span>
          </span>
          <Tooltip content="Collapse sidebar" side="bottom">
            <TitlebarButton
              label="Collapse sidebar"
              onClick={toggleRail}
              className="titlebar-no-drag"
            >
              <ChevronsLeft className="size-3.5" />
            </TitlebarButton>
          </Tooltip>
        </>
      ) : (
        <Tooltip content="Expand sidebar" side="bottom">
          <TitlebarButton
            label="Expand sidebar"
            onClick={toggleRail}
            className="titlebar-no-drag group relative"
          >
            <img
              src="./assets/logo-64.png"
              alt=""
              className="size-3.5 transition-opacity duration-150 ease-out group-hover:opacity-0 group-focus-visible:opacity-0"
            />
            <ChevronsRight
              aria-hidden="true"
              className="absolute size-3.5 opacity-0 transition-opacity duration-150 ease-out group-hover:opacity-100 group-focus-visible:opacity-100"
            />
          </TitlebarButton>
        </Tooltip>
      )}
    </div>
  );
}

/**
 * A thread list behind its own disclosure. The heading is the target - the
 * chevron alone is a 12px hit area, and the row is already the width of the
 * rail - so the whole thing is one button with the chevron trailing on the
 * right edge, on the label's centre line.
 *
 * Collapsed, the count moves into view: the heading has to keep saying how
 * much is hidden behind it, or the list looks empty rather than folded.
 */
function RailSection({ label, count, open, onToggle, className, children }) {
  const id = `rail-section-${label.toLowerCase()}`;
  return (
    <div className={cn("flex flex-col", className)}>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        aria-controls={id}
        className={cn(
          // rounded-xl and h-7, so the hover fill is the same shape as the
          // thread rows underneath it rather than a second, flatter pill.
          "group flex h-7 w-full items-center gap-1.5 rounded-xl px-2 text-left",
          "outline-none transition-colors duration-150 ease-out",
          "hover:fill-nav focus-visible:fill-nav",
        )}
      >
        <span
          className={cn(
            "text-[11px] font-semibold text-muted-foreground",
            "transition-colors duration-150 ease-out group-hover:text-foreground",
          )}
        >
          {label}
        </span>
        {!open && count > 0 ? (
          <span className="text-[11px] tabular-nums text-muted-foreground/60">
            {count}
          </span>
        ) : null}
        <ChevronDown
          aria-hidden="true"
          className={cn(
            "ml-auto size-3.5 shrink-0 text-muted-foreground/50",
            "transition-[transform,color] duration-200 ease-out",
            "group-hover:text-foreground",
            !open && "-rotate-90",
          )}
        />
      </button>
      <Collapse open={open} id={id} innerClassName="mt-0.5 flex flex-col gap-0.5">
        {children}
      </Collapse>
    </div>
  );
}

/**
 * How a label fades beside its icon when the rail opens and closes.
 *
 * Only opacity and a few pixels of travel are animated. The width is not,
 * on purpose: it snaps to nothing when the rail collapses, so the icon column
 * lands on its centre the same frame the column starts narrowing, rather than
 * being dragged along behind a label that is still easing away.
 */
const RAIL_REVEAL = "transition-[opacity,transform] duration-[var(--motion-base)] ease-[var(--ease-out)]";
const RAIL_SHOWN = "translate-x-0 opacity-100";
const RAIL_HIDDEN = "-translate-x-1 opacity-0";

/**
 * Everything to the right of a rail icon - the label, a shortcut, a count -
 * kept mounted in both states so it can fade rather than pop. Collapsed it is
 * zero width and clipped, which is what lets the icon beside it stay centred.
 */
function RailLabel({ expanded, className, children }) {
  return (
    <span
      className={cn(
        "flex min-w-0 items-center gap-2",
        RAIL_REVEAL,
        expanded ? cn("flex-1", RAIL_SHOWN) : cn("w-0 overflow-hidden", RAIL_HIDDEN),
        className,
      )}
    >
      {children}
    </span>
  );
}

function RailHeading({ expanded, children, className }) {
  return (
    <div
      className={cn(
        "px-2 text-[11px] font-semibold text-muted-foreground",
        RAIL_REVEAL,
        // Height snaps for the same reason a label's width does: the rows
        // underneath must not drift while the heading eases out.
        expanded ? cn("pb-1", RAIL_SHOWN) : cn("h-0 overflow-hidden", RAIL_HIDDEN),
        className,
      )}
    >
      {children}
    </div>
  );
}

/**
 * The three row controls share one look: a round target that is not merely
 * transparent at rest but zero-width, so it takes no space until the row is
 * hovered. That is what lets a pinned row's pin sit flush against the right
 * edge and then glide left as its neighbours open, rather than everything
 * fading in on top of a layout that never moved.
 */
const ROW_ACTION = cn(
  "flex h-6 w-0 shrink-0 items-center justify-center overflow-hidden rounded-full",
  "text-muted-foreground opacity-0 outline-none",
  "transition-[width,opacity,color,background-color] duration-200 ease-out",
  "hover:fill-control-hover hover:text-foreground",
  "focus-visible:w-6 focus-visible:opacity-100 focus-visible:fill-control-hover",
  "group-hover/row:w-6 group-hover/row:opacity-100",
);

/**
 * The face on a conversation row.
 *
 * The same one the Library shows, which is the point: a conversation is with an
 * agent, and recognising which one should not depend on which screen you are
 * looking at. A bare `Avatar` drew initials only - no picture, no tint - so two
 * teammates whose names began with the same letter were the same grey circle.
 *
 * `AgentAvatar` renders nothing at all when the agent is missing, which would
 * let the title slide left and break the column the rows share. A deleted agent
 * still leaves its conversations behind, so that case gets a plain circle and
 * the row keeps its shape.
 */
function ThreadFace({ agent }) {
  if (!agent) return <Avatar size="xs" className="shrink-0" />;
  return <AgentAvatar agent={agent} size="xs" className="shrink-0" />;
}

function ThreadRow({
  thread,
  agent,
  active,
  fresh,
  onOpen,
  onTogglePin,
  onRename,
  onDelete,
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(thread.title);
  const [confirming, setConfirming] = useState(false);
  const inputRef = useRef(null);

  useEffect(() => {
    if (!editing) return;
    // A frame later, or the input is not in the tree yet to focus.
    const frame = requestAnimationFrame(() => {
      setDraft(thread.title);
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    return () => cancelAnimationFrame(frame);
  }, [editing, thread.title]);

  const commit = () => {
    const next = draft.trim();
    setEditing(false);
    if (next && next !== thread.title) onRename(thread.id, next);
  };

  if (editing) {
    return (
      <div className="flex h-8 items-center gap-2 rounded-xl fill-control px-2">
        <ThreadFace agent={agent} />
        <input
          ref={inputRef}
          value={draft}
          maxLength={120}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              commit();
            }
            if (event.key === "Escape") setEditing(false);
          }}
          aria-label="Rename conversation"
          className="min-w-0 flex-1 bg-transparent text-[13px] text-foreground outline-none"
        />
        <button
          type="button"
          // Mousedown would blur the input first, and blur commits - so the
          // click would land on a row that had already stopped editing.
          onMouseDown={(event) => event.preventDefault()}
          onClick={commit}
          aria-label="Save name"
          className="shrink-0 rounded-full p-1 text-muted-foreground outline-none transition-colors duration-150 ease-out hover:text-foreground focus-visible:fill-control-hover"
        >
          <Check className="size-3.5" />
        </button>
      </div>
    );
  }

  return (
    <>
      <div
        data-active={active || undefined}
        className={cn(
          "group/row flex h-8 items-center gap-2 rounded-xl pl-2 pr-1",
          "transition-colors duration-150 ease-out",
          "hover:fill-nav data-[active=true]:fill-nav-active",
          // A row that was not here a moment ago slides in; the rest were.
          fresh && "animate-slide-up",
        )}
      >
        <button
          type="button"
          onClick={() => onOpen(thread.id)}
          title={thread.title}
          className="flex min-w-0 flex-1 items-center gap-2 text-left outline-none focus-visible:fill-nav rounded-lg"
        >
          <ThreadFace agent={agent} />
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-[13px]",
              active ? "font-medium text-foreground" : "text-foreground/90",
            )}
          >
            {thread.title}
          </span>
        </button>

        {/* The timestamp collapses on the same curve the actions open on, so
            the row's right edge stays still and one thing simply becomes the
            other. Fading it in place would leave the actions landing on top of
            text that was still there. */}
        <span
          aria-hidden={true}
          className={cn(
            "shrink-0 overflow-hidden whitespace-nowrap text-[10px] text-muted-foreground/70",
            "transition-[width,opacity] duration-200 ease-out",
            "group-hover/row:w-0 group-hover/row:opacity-0",
          )}
        >
          {thread.unread > 0 ? (
            <Badge size="sm">{thread.unread}</Badge>
          ) : (
            relativeTime(thread.updatedAt)
          )}
        </span>

        {/* The actions are their own cluster: the row's gap belongs between
            avatar, title and timestamp, not between three targets that read as
            one control. */}
        <div className="flex shrink-0 items-center">
          {/* A pinned row keeps its pin open at rest. Otherwise there is
              nothing on the row to say why it is sitting under Pinned. */}
          <button
            type="button"
            aria-label={thread.pinned ? "Unpin conversation" : "Pin conversation"}
            className={cn(ROW_ACTION, thread.pinned && "w-6 opacity-100")}
            onClick={() => onTogglePin(thread.id)}
          >
            {thread.pinned ? (
              <PinOff className="size-3.5 shrink-0" />
            ) : (
              <Pin className="size-3.5 shrink-0" />
            )}
          </button>
          <button
            type="button"
            aria-label="Rename conversation"
            className={ROW_ACTION}
            onClick={() => setEditing(true)}
          >
            <Pencil className="size-3.5 shrink-0" />
          </button>
          {/* Destructive last, so it is never the neighbour of a mis-click. */}
          <button
            type="button"
            aria-label="Delete conversation"
            className={cn(ROW_ACTION, "hover:text-destructive-ink")}
            onClick={() => setConfirming(true)}
          >
            <Trash2 className="size-3.5 shrink-0" />
          </button>
        </div>
      </div>

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title="Delete this conversation?"
        description={`"${thread.title}" and its messages will be removed from the workspace folder. This cannot be undone.`}
        confirmLabel="Delete"
        destructive
        onConfirm={() => onDelete(thread.id)}
      />
    </>
  );
}

function RailRow({
  expanded,
  icon: Icon,
  label,
  active,
  count,
  trailing,
  emphasis,
  onClick,
}) {
  const row = (
    <button
      type="button"
      onClick={onClick}
      data-active={active || undefined}
      aria-current={active ? "page" : undefined}
      className={cn(
        "group flex h-8 w-full items-center rounded-xl text-left",
        "outline-none transition-colors duration-150 ease-out",
        "hover:fill-nav focus-visible:fill-nav",
        "data-[active=true]:fill-nav-active",
        // No gap when collapsed: the label is still there at zero width, and
        // a gap beside it would nudge the icon off centre.
        expanded ? "gap-2 px-2" : "justify-center gap-0 px-0",
      )}
    >
      <span className="flex size-5 shrink-0 items-center justify-center">
        <Icon
          className={cn(
            "size-4 transition-colors duration-150 ease-out",
            // the current view is the one place in the rail an accent earns
            // its keep; emphasis is loud already and stays ink
            active
              ? "accent-ink"
              : emphasis
                ? "text-foreground"
                : "text-muted-foreground group-hover:text-foreground",
          )}
        />
      </span>
      <RailLabel expanded={expanded}>
        <span
          className={cn(
            "min-w-0 flex-1 truncate text-[13.5px]",
            active ? "accent-ink font-medium" : "text-foreground/90",
          )}
        >
          {label}
        </span>
        {trailing}
        {count > 0 && <Badge size="sm">{count}</Badge>}
      </RailLabel>
    </button>
  );

  if (expanded) return row;
  return (
    <Tooltip content={label} side="right">
      {row}
    </Tooltip>
  );
}

function ThemeRow({ expanded }) {
  const { theme, resolved, setTheme } = useTheme();
  const Icon = theme === "system" ? SunMoon : resolved === "dark" ? Moon : Sun;

  return (
    <DropdownMenu
      side="right"
      align="end"
      trigger={
        <button
          type="button"
          className={cn(
            "group flex h-8 w-full items-center rounded-xl text-left",
            "outline-none transition-colors duration-150 ease-out",
            "hover:fill-nav focus-visible:fill-nav",
            "data-[state=open]:bg-muted",
            expanded ? "gap-2 px-2" : "justify-center gap-0 px-0",
          )}
          aria-label="Theme"
        >
          <span className="flex size-5 shrink-0 items-center justify-center">
            <Icon className="size-4 text-muted-foreground group-hover:text-foreground" />
          </span>
          <RailLabel expanded={expanded}>
            <span className="min-w-0 flex-1 truncate text-[13.5px] text-foreground/90">
              Theme
            </span>
            <span className="text-[11px] capitalize text-muted-foreground">
              {theme}
            </span>
          </RailLabel>
        </button>
      }
    >
      <MenuLabel>Appearance</MenuLabel>
      <MenuGroup>
        <MenuItem
          icon={SunMoon}
          checked={theme === "system"}
          onSelect={() => setTheme("system")}
        >
          System
        </MenuItem>
        <MenuItem
          icon={Sun}
          checked={theme === "light"}
          onSelect={() => setTheme("light")}
        >
          Light
        </MenuItem>
        <MenuItem
          icon={Moon}
          checked={theme === "dark"}
          onSelect={() => setTheme("dark")}
        >
          Dark
        </MenuItem>
      </MenuGroup>
    </DropdownMenu>
  );
}

function UserRow({ expanded, user, onSettings }) {
  // Read from the workspace file rather than the record, so replacing
  // settings/avatar.png in the folder shows up here too.
  const avatarSrc = useAvatarSrc(user);
  const ref = useRef(null);
  return (
    <DropdownMenu
      side="right"
      align="end"
      panelClassName="w-56"
      trigger={
        <button
          ref={ref}
          type="button"
          aria-label="Account"
          className={cn(
            "group flex h-10 w-full items-center rounded-xl text-left",
            "outline-none transition-colors duration-150 ease-out",
            "hover:fill-nav focus-visible:fill-nav",
            "data-[state=open]:bg-muted",
            expanded ? "gap-2 px-2" : "justify-center gap-0 px-0",
          )}
        >
          <span className="flex size-5 shrink-0 items-center justify-center">
            <Avatar size="xs" src={avatarSrc ?? undefined} name={user.name} />
          </span>
          <RailLabel expanded={expanded}>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[13px] font-medium text-foreground">
                {user.name}
              </span>
              <span className="block truncate text-[11px] text-muted-foreground">
                {user.handle ?? user.email}
              </span>
            </span>
          </RailLabel>
        </button>
      }
    >
      <div className="flex items-center gap-2 px-2 py-2">
        <Avatar size="md" src={avatarSrc ?? undefined} name={user.name} />
        <div className="min-w-0">
          <div className="truncate text-[13px] font-medium">{user.name}</div>
          <div className="truncate text-[11px] text-muted-foreground">
            {user.email}
          </div>
        </div>
      </div>
      <MenuSeparator />
      <MenuGroup>
        <MenuItem icon={Settings} onSelect={() => onSettings("general")}>
          Settings
        </MenuItem>
        <MenuItem icon={User} onSelect={() => onSettings("identity")}>
          Identity
        </MenuItem>
        <MenuItem icon={Command} onSelect={() => onSettings("shortcuts")}>
          Keyboard shortcuts
        </MenuItem>
      </MenuGroup>
    </DropdownMenu>
  );
}
