import * as React from "react";
import {
  Bell,
  Brain,
  Cpu,
  FolderTree,
  Info,
  KeyRound,
  Keyboard,
  Mic,
  Palette,
  Plug,
  ShieldCheck,
  SlidersHorizontal,
  UserRound,
  Users,
  Workflow,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { useAvatarSrc } from "@/lib/avatar";
import { Avatar } from "@/components/ui/avatar";
import { Dialog } from "@/components/ui/dialog";
import { Tabs } from "@/components/ui/tabs";

import { GeneralPane } from "./panes/GeneralPane";
import { AppearancePane } from "./panes/AppearancePane";
import { IdentityPane } from "./panes/IdentityPane";
import { ModelsPane } from "./panes/ModelsPane";
import { VoicePane } from "./panes/VoicePane";
import { ComputersPane } from "./panes/ComputersPane";
import { PermissionsPane } from "./panes/PermissionsPane";
import { HooksPane } from "./panes/HooksPane";
import { ShortcutsPane } from "./panes/ShortcutsPane";
import { WorkspacePane } from "./panes/WorkspacePane";
import { SecretsPane } from "./panes/SecretsPane";
import { MemoryPane } from "./panes/MemoryPane";
import { GroupPane } from "./panes/GroupPane";
import { NotificationsPane } from "./panes/NotificationsPane";
import { AboutPane } from "./panes/AboutPane";

/** Tab ids match the `settings.tab` values the store holds. */
const GROUPS = [
  {
    id: "you",
    label: null,
    tabs: [
      { value: "general", label: "General", icon: SlidersHorizontal, Pane: GeneralPane },
      { value: "identity", label: "Identity", icon: UserRound, Pane: IdentityPane },
      { value: "appearance", label: "Appearance", icon: Palette, Pane: AppearancePane },
    ],
  },
  {
    id: "intelligence",
    label: "Intelligence",
    tabs: [
      { value: "models", label: "Providers", icon: Plug, Pane: ModelsPane },
      { value: "voice", label: "Voice", icon: Mic, Pane: VoicePane },
      { value: "computers", label: "Computers", icon: Cpu, Pane: ComputersPane },
      { value: "memory", label: "Memory", icon: Brain, Pane: MemoryPane },
      { value: "group", label: "Agent Group", icon: Users, Pane: GroupPane },
    ],
  },
  {
    id: "safety",
    label: "Safety",
    tabs: [
      { value: "permissions", label: "Permissions", icon: ShieldCheck, Pane: PermissionsPane },
      { value: "hooks", label: "Hooks", icon: Workflow, Pane: HooksPane },
    ],
  },
  {
    id: "app",
    label: "Application",
    tabs: [
      { value: "notifications", label: "Notifications", icon: Bell, Pane: NotificationsPane },
      { value: "shortcuts", label: "Shortcuts", icon: Keyboard, Pane: ShortcutsPane },
      { value: "workspace", label: "Workspace", icon: FolderTree, Pane: WorkspacePane },
      { value: "secrets", label: "Secrets", icon: KeyRound, Pane: SecretsPane },
      { value: "about", label: "About", icon: Info, Pane: AboutPane },
    ],
  },
];

const TABS = GROUPS.flatMap((g) => g.tabs);

function NavRow({ tab, active, onSelect }) {
  const Icon = tab.icon;
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      data-state={active ? "active" : "inactive"}
      onClick={() => onSelect(tab.value)}
      className={cn(
        "flex h-7 w-full shrink-0 items-center gap-1.5 rounded-lg px-2 text-left text-xs outline-none",
        "transition-colors duration-75 ease-out",
        "focus-visible:fill-nav",
        active
          ? "bg-muted font-medium text-foreground"
          : "text-muted-foreground hover:fill-nav hover:text-foreground"
      )}
    >
      <Icon className="size-3.5 shrink-0" aria-hidden="true" />
      <span className="truncate">{tab.label}</span>
    </button>
  );
}

export function SettingsDialog() {
  const { settings, closeSettings, setSettingsTab, user } = useApp();
  const avatarSrc = useAvatarSrc(user);
  const navRef = React.useRef(null);
  const scrollRef = React.useRef(null);

  const active = TABS.find((t) => t.value === settings.tab) ?? TABS[0];
  const Pane = active.Pane;

  // a deep-linked tab must not open half-cut at the edge of the mobile strip
  React.useEffect(() => {
    if (!settings.open) return;
    navRef.current
      ?.querySelector('[data-state="active"]')
      ?.scrollIntoView({ inline: "nearest", block: "nearest" });
  }, [settings.open, settings.tab]);

  // switching panes should start at the top, not where the last pane was left
  React.useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [settings.tab]);

  return (
    <Dialog
      open={settings.open}
      onOpenChange={(next) => {
        if (!next) closeSettings();
      }}
      size="full"
      showClose
      ariaLabel="Settings"
      className={cn(
        "w-[calc(100vw-2rem)] sm:w-[calc(100vw-3rem)] lg:w-[calc(100vw-4rem)]",
        "max-w-[80rem] sm:max-w-[80rem]",
        "h-[min(54rem,calc(100dvh-4rem))] max-h-[calc(100dvh-4rem)]",
        "gap-0 overflow-hidden rounded-3xl bg-background p-0 md:flex-row"
      )}
    >
      {/* ── nav column: 240px at md, a scrolling strip below ─────────────── */}
      <div className="flex shrink-0 flex-col gap-0 md:w-[15rem] md:min-h-0">
        {/* The mark, not a Back button: this dialog covers the app rather than
            being a place inside it, so the corner should say where you are.
            Dismissal is the panel's own close control, top right. */}
        <div className="flex items-center gap-2 px-4 pt-3.5 md:pt-4">
          <img src="./assets/logo-64.png" alt="" className="size-5 shrink-0" />
          <div className="min-w-0 flex-1">
            <p className="truncate text-xs font-medium text-foreground">Inertia</p>
            <p className="truncate text-[0.6875rem] leading-tight text-muted-foreground">
              Settings
            </p>
          </div>
        </div>

        {/* below md: one horizontally scrolling strip with the edge fade */}
        <div ref={navRef} className="min-h-0 md:hidden">
          <Tabs
            value={active.value}
            onChange={setSettingsTab}
            variant="strip"
            ariaLabel="Settings sections"
            idPrefix="settings"
            items={TABS.map((t) => ({ value: t.value, label: t.label, icon: t.icon }))}
            className="px-3 py-2"
          />
        </div>

        {/* md and up: the grouped column */}
        <div
          role="tablist"
          aria-label="Settings sections"
          aria-orientation="vertical"
          className="hidden min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto no-scrollbar px-2 pt-2 pb-1 md:flex"
        >
          {GROUPS.map((group, i) => (
            <React.Fragment key={group.id}>
              {group.label ? (
                <p
                  className={cn(
                    "px-2 pb-0.5 text-[0.625rem] text-muted-foreground",
                    i === 0 ? "pt-1.5" : "pt-2.5"
                  )}
                >
                  {group.label}
                </p>
              ) : null}
              {group.tabs.map((tab) => (
                <NavRow
                  key={tab.value}
                  tab={tab}
                  active={tab.value === active.value}
                  onSelect={setSettingsTab}
                />
              ))}
            </React.Fragment>
          ))}
        </div>

        {/* footer: who is signed in */}
        <div className="hidden shrink-0 items-center gap-2 px-4 pb-3.5 md:flex">
          <Avatar
            src={avatarSrc ?? undefined}
            name={user.avatarInitials ?? user.name}
            size="sm"
          />
          <div className="min-w-0 flex-1">
            <p className="truncate text-xs text-foreground/90">{user.name}</p>
            <p className="truncate text-[0.6875rem] leading-tight text-muted-foreground">
              {user.email}
            </p>
          </div>
        </div>
      </div>

      {/* ── content pane: padding lives on the heading and the scroll box ── */}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col py-4">
        <h2 className="mb-3 shrink-0 px-4 text-base font-medium text-foreground md:pr-7 md:pl-5">
          {active.label}
        </h2>
        <div
          ref={scrollRef}
          id={`settings-panel-${active.value}`}
          role="tabpanel"
          aria-label={active.label}
          tabIndex={-1}
          className="min-h-0 flex-1 overflow-y-auto no-scrollbar px-4 pb-4 outline-none md:pr-7 md:pl-5"
        >
          {/* Keyed on the tab so a new pane is a new element and fades in,
              rather than the old one's rows being rewritten in place. */}
          <div key={active.value} className="animate-fade-in">
            <Pane />
          </div>
        </div>
      </div>
    </Dialog>
  );
}
