import * as React from "react";
import { useApp } from "@/lib/store";
import { useWorkspace } from "@/lib/workspace";
import { DEFAULT_THEME, useTheme } from "@/lib/theme";
import { PREF, RESUME_LAST, writePref } from "@/lib/persist";
import { PREFERENCE_DEFAULTS } from "@/lib/appearance";
import { MODES } from "@shared/modes";
import { APPROVALS } from "@shared/approval";
import { TOOL_ACCESS } from "@shared/tool-access";
import {
  LOCALES,
  SYSTEM,
  TIME_ZONES,
  formatDate,
  formatDateTime,
  formatNumber,
  resolveLocale,
  resolveTimeZone,
  zoneOffsetLabel,
} from "@/lib/datetime";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { useBackgroundSettings } from "@/hooks/use-background-settings";
import { SettingsCard, SettingsRow, SettingsSection } from "../SettingsRow";

/**
 * The views the shell can be told to open on.
 *
 * `RESUME_LAST` first because it is what the app has always done, and because
 * "where you left off" is the only entry that is not a place.
 */
const STARTUP_VIEWS = [
  {
    value: RESUME_LAST,
    label: "Where you left off",
    description: "The screen you closed the window on",
  },
  { value: "chat", label: "Chat" },
  { value: "library", label: "Library" },
  { value: "agents", label: "Agents" },
  { value: "computers", label: "Computers" },
  { value: "routines", label: "Routines" },
  { value: "memory", label: "Memory" },
  { value: "integrations", label: "Integrations" },
  { value: "activity", label: "Activity" },
];

/** Typed by hand into the second delete dialog. Case sensitive on purpose: a
 *  word you have to reach for the shift key to write is a word you meant. */
const CONFIRM_WORD = "DELETE";

/** A sample every locale row is measured against, so the dropdown shows what
 *  the choice actually does rather than the name of a country. */
const SAMPLE_NUMBER = 1234.5;

export function GeneralPane() {
  const {
    user,
    agents,
    threads,
    routines,
    computers,
    memories,
    setPreference,
    resetPreferences,
    openSettings,
  } = useApp();
  const { setTheme } = useTheme();
  const { toast } = useToast();
  const { root, wipe } = useWorkspace();
  const background = useBackgroundSettings();

  // Named after the machine the person is sitting at. "Start when Windows
  // starts" on a Mac is the kind of small wrongness that makes somebody wonder
  // what else the app has not noticed about their computer.
  const startupLabel =
    background.platform === "darwin"
      ? "Open Inertia at login"
      : "Start when Windows starts";
  // Closing to the tray is a Windows and Linux idea. macOS already keeps a
  // closed app in the dock, so the switch would describe something the platform
  // decides, and there is no tray icon there to close to.
  const showTrayRow = background.available && background.platform !== "darwin";

  const prefs = user.preferences;
  const locale = prefs.locale ?? PREFERENCE_DEFAULTS.locale;
  const timezone = prefs.timezone ?? PREFERENCE_DEFAULTS.timezone;
  // An unset startup view is not the same as choosing Chat: it means nobody has
  // ever answered the question, and the honest answer to "where does this open"
  // is then whatever the shell already does.
  const startupView = prefs.startupView ?? RESUME_LAST;

  const [confirm, setConfirm] = React.useState(null); // 'reset' | 'delete' | 'delete-final'
  const [typed, setTyped] = React.useState("");
  const [wiping, setWiping] = React.useState(false);

  // The clock in the example row has to move, or the setting looks like a label
  // rather than a live formatting of the current instant. Half a minute is
  // often enough for a minute-precision string to never look stale.
  const [now, setNow] = React.useState(() => new Date());
  React.useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

  // The mirror the shell reads at first render - see `startupOverride` in
  // lib/persist. The pane is the only writer, because it is the only place the
  // preference can change.
  React.useEffect(() => {
    writePref(PREF.startupView, prefs.startupView ?? RESUME_LAST);
  }, [prefs.startupView]);

  const agentOptions = React.useMemo(
    () =>
      agents.map((b) => ({ value: b.id, label: b.name, description: b.role })),
    [agents],
  );

  const localeOptions = React.useMemo(
    () =>
      LOCALES.map((entry) => {
        const preferences = { locale: entry.value, timezone };
        return {
          ...entry,
          label:
            entry.value === SYSTEM
              ? `Match system (${resolveLocale(SYSTEM)})`
              : entry.label,
          description: `${formatDate(now, preferences)} · ${formatNumber(SAMPLE_NUMBER, preferences)}`,
        };
      }),
    [now, timezone],
  );

  const zoneOptions = React.useMemo(
    () =>
      TIME_ZONES.map((entry) => ({
        ...entry,
        label:
          entry.value === SYSTEM
            ? `Match system (${resolveTimeZone(SYSTEM)})`
            : entry.label,
        description: zoneOffsetLabel(entry.value, now),
      })),
    [now],
  );

  const example = formatDateTime(now, { locale, timezone });

  function resetEverything() {
    // One write, to the one list of defaults. The pane used to name a dozen
    // keys by hand, which is how it ended up resetting to values the app had
    // not shipped for months.
    resetPreferences();
    setTheme(DEFAULT_THEME);
    setConfirm(null);
    toast({
      title: "Settings reset",
      description: "Every preference is back to its shipped default.",
      variant: "success",
    });
  }

  async function deleteEverything() {
    setWiping(true);
    try {
      await wipe();
      setConfirm(null);
      setTyped("");
      toast({
        title: "Everything deleted",
        description: "The workspace folder is empty and ready to start again.",
        variant: "success",
      });
    } catch (error) {
      // The real message, not a friendly one: a delete that failed halfway is
      // exactly the moment somebody needs to know which path refused.
      toast({
        title: "Nothing was deleted",
        description:
          error?.message ?? "The workspace folder could not be cleared.",
        variant: "danger",
      });
    } finally {
      setWiping(false);
    }
  }

  const closeDelete = () => {
    if (wiping) return;
    setConfirm(null);
    setTyped("");
  };

  return (
    <div className="w-full">
      <SettingsSection title="You">
        <SettingsRow
          label={user.name}
          description={`${user.handle} · ${user.email}`}
          control={
            <Button
              variant="subtle"
              size="xs"
              onClick={() => openSettings("identity")}
            >
              Edit profile
            </Button>
          }
        />
      </SettingsSection>

      <SettingsSection title="Region">
        <SettingsRow
          label="Date and number format"
          description="Which order dates are written in, whether the clock runs to 12 or 24, and how thousands are separated. The interface itself is English only for now, and agent replies follow the language you write in."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Date and number format"
              value={locale}
              onChange={(v) => setPreference("locale", v)}
              options={localeOptions}
            />
          }
        />
        <SettingsRow
          label="Time zone"
          description="Every date and time the app writes out is rendered in this zone."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Time zone"
              value={timezone}
              onChange={(v) => setPreference("timezone", v)}
              options={zoneOptions}
            />
          }
        />
        <SettingsRow
          label="Right now"
          description="The current time, written with the two settings above."
          control={
            <span className="font-mono text-xs text-foreground tabular-nums">
              {example}
            </span>
          }
        />
      </SettingsSection>

      <SettingsSection title="Behaviour">
        <SettingsRow
          label="Send on Enter"
          htmlFor="set-send-on-enter"
          description="Off means Enter inserts a newline and ⌘/Ctrl + Enter sends."
          control={
            <Switch
              id="set-send-on-enter"
              size="sm"
              label="Send on Enter"
              checked={!!prefs.sendOnEnter}
              onCheckedChange={(v) => setPreference("sendOnEnter", v)}
            />
          }
        />
        <SettingsRow
          label="Default agent for new chats"
          description="Which teammate the New chat button starts a thread with."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Default agent for new chats"
              value={prefs.defaultAgentId ?? agents[0]?.id}
              onChange={(v) => setPreference("defaultAgentId", v)}
              options={agentOptions}
            />
          }
        />
        {/* The composer's two pills, as defaults. Each new chat starts with
            these and its own pills still change it; this is for the person
            who sets the same two things at the start of every conversation. */}
        <SettingsRow
          label="Default mode for new chats"
          description="Decides which tools a new conversation holds, and whether it is one agent or a room of them; the pill in the composer still changes it per chat."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Default mode for new chats"
              value={prefs.defaultMode ?? PREFERENCE_DEFAULTS.defaultMode}
              onChange={(v) => setPreference("defaultMode", v)}
              options={MODES.map((m) => ({ value: m.id, label: m.label, description: m.hint }))}
            />
          }
        />
        <SettingsRow
          label="Default approval for new chats"
          description="Whether a new conversation asks before acting. Deny rules still hold whatever this says."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Default approval for new chats"
              value={prefs.defaultApproval ?? PREFERENCE_DEFAULTS.defaultApproval}
              onChange={(v) => setPreference("defaultApproval", v)}
              options={APPROVALS.map((a) => ({ value: a.id, label: a.label, description: a.hint }))}
            />
          }
        />
        {/* The one setting that trades latency against how much a turn knows
            about itself. Next to the mode and approval pills because it is the
            same kind of choice: what a new conversation starts holding. */}
        <SettingsRow
          label="Tool access"
          description="Whether every tool is loaded into each turn, or only the ones used constantly, with the rest loaded on request. Loading on request makes replies start sooner and uses less context; the agent can still reach everything."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Tool access"
              value={prefs.toolAccess ?? PREFERENCE_DEFAULTS.toolAccess}
              onChange={(v) => setPreference("toolAccess", v)}
              options={TOOL_ACCESS.map((t) => ({ value: t.id, label: t.label, description: t.hint }))}
            />
          }
        />
        <SettingsRow
          label="Startup view"
          description="Where Inertia lands when the window opens."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Startup view"
              value={startupView}
              onChange={(v) => setPreference("startupView", v)}
              options={STARTUP_VIEWS}
            />
          }
        />
      </SettingsSection>

      {/* Hidden entirely when there is no backend to ask - a browser tab
          or a test render - rather than shown as two switches that do nothing. */}
      {background.available ? (
        <SettingsSection title="Startup and background">
          {showTrayRow ? (
            <SettingsRow
              label="Keep running in the background when the window is closed"
              htmlFor="set-minimise-to-tray"
              description="Closing the window hides it and leaves Inertia in the notification area, so agents keep working and the next open is instant. Off means closing the window quits - though a turn that is still running will hold the app open until it finishes."
              control={
                <Switch
                  id="set-minimise-to-tray"
                  size="sm"
                  label="Keep running in the background when the window is closed"
                  disabled={background.loading}
                  checked={background.minimiseToTray}
                  onCheckedChange={(v) => background.setMinimiseToTray(v)}
                />
              }
            />
          ) : null}
          {background.launchAtLoginSupported ? (
            <SettingsRow
              label={startupLabel}
              htmlFor="set-launch-at-login"
              description="Launches Inertia when you sign in, so routines and scheduled work do not wait for you to remember."
              control={
                <Switch
                  id="set-launch-at-login"
                  size="sm"
                  label={startupLabel}
                  disabled={background.loading}
                  checked={background.launchAtLogin}
                  onCheckedChange={(v) => background.setLaunchAtLogin(v)}
                />
              }
            />
          ) : null}
        </SettingsSection>
      ) : null}

      <SettingsSection flat title="Privacy">
        {/* There was a telemetry switch here. Nothing in the app has ever
            collected or sent a single event, so the switch was a promise made
            about a system that does not exist - and a promise is worse than
            silence. A statement of fact replaces it. */}
        <SettingsCard>
          <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
            Inertia has no analytics, no crash reporter and no account. Nothing
            about how you use the app leaves this computer. The only outbound
            traffic is what you ask for: requests to the model providers and
            integrations you configured yourself, sent straight from here with
            your own keys. Your conversations, agents and memories are files in
            your workspace folder, and they stay there.
          </p>
        </SettingsCard>
      </SettingsSection>

      <SettingsSection title="Danger zone">
        <SettingsRow
          label="Reset all settings"
          description="Puts every preference in this sheet back to its default. Your threads, agents, memories and computers are untouched."
          control={
            <Button
              variant="subtle"
              size="xs"
              onClick={() => setConfirm("reset")}
            >
              Reset
            </Button>
          }
        />
        <SettingsRow
          label="Delete all data"
          description="Permanently removes everything Inertia has written into your workspace folder: every agent and its memory, all threads and attachments, every routine, computer and skill. This cannot be undone."
          control={
            <Button
              variant="danger"
              size="xs"
              onClick={() => setConfirm("delete")}
            >
              Delete all data
            </Button>
          }
        />
      </SettingsSection>

      <ConfirmDialog
        open={confirm === "reset"}
        onOpenChange={(v) => !v && setConfirm(null)}
        destructive
        title="Reset all settings?"
        description="Theme, accent, density, font size, date format, time zone and every toggle in this sheet return to their shipped values. Threads, agents, memories, routines and computers are not affected."
        confirmLabel="Reset settings"
        onConfirm={resetEverything}
      />

      {/* Step one: what goes, and where it lives. No confirmation word here -
          the first dialog exists to be read, and a text field to fill in gives
          somebody something to do instead. */}
      <ConfirmDialog
        open={confirm === "delete"}
        onOpenChange={(v) => !v && closeDelete()}
        destructive
        title="Delete everything in your workspace?"
        description={
          <>
            This removes every agent and everything it has learned, every
            conversation and attachment, every routine, every computer, every
            skill, the record of which integrations you connected, and your
            profile. There is no export step and no undo: once the files are
            gone, they are gone. It all lives in this folder.
            <span className="mt-2 block font-mono text-[0.6875rem] break-all text-foreground/80">
              {root ?? "No workspace folder is configured."}
            </span>
          </>
        }
        confirmLabel="Continue"
        onConfirm={() => setConfirm("delete-final")}
      />

      {/* Step two: what it costs in numbers, what survives, and a word to type.
          Counting the records is the point - "everything" is abstract, and
          "31 conversations" is not. */}
      <Dialog
        open={confirm === "delete-final"}
        onOpenChange={(v) => !v && closeDelete()}
        size="sm"
        showClose={false}
        closeOnOverlay={!wiping}
        ariaLabel="Confirm deleting all data"
        className="w-[calc(100%-1.5rem)]"
      >
        <DialogTitle>Last check</DialogTitle>
        <DialogDescription>
          About to be deleted from{" "}
          <span className="font-mono text-[0.6875rem] break-all text-foreground/80">
            {root ?? "the workspace folder"}
          </span>
          :
        </DialogDescription>

        <DialogBody className="mt-3">
          <dl className="flex flex-col gap-1 text-xs">
            {[
              ["Agents, with their memories", agents.length],
              ["Conversations", threads.length],
              ["Memories", memories.length],
              ["Routines", routines.length],
              ["Computers", computers.length],
            ].map(([label, count]) => (
              <div
                key={label}
                className="flex items-baseline justify-between gap-3"
              >
                <dt className="text-muted-foreground">{label}</dt>
                <dd className="tabular-nums text-foreground">{count}</dd>
              </div>
            ))}
          </dl>

          <p className="mt-3 text-[0.6875rem] leading-relaxed text-muted-foreground">
            The folder itself stays where it is and stays connected, so Inertia
            reopens empty rather than back at setup. Anything in there that
            Inertia did not write is left alone. Your API keys and provider
            settings live outside the workspace and are not touched.
          </p>

          <label
            htmlFor="wipe-confirm"
            className="mt-4 block text-xs text-foreground/90"
          >
            Type {CONFIRM_WORD} to confirm
          </label>
          <Input
            id="wipe-confirm"
            size="sm"
            className="mt-1.5"
            value={typed}
            autoComplete="off"
            spellCheck={false}
            disabled={wiping}
            // No placeholder: a greyed-out DELETE sitting in the field reads as
            // already typed, which is the one thing this control must not do.
            onChange={(e) => setTyped(e.target.value)}
          />
        </DialogBody>

        <DialogFooter>
          <Button
            variant="secondary"
            size="pill"
            disabled={wiping}
            onClick={closeDelete}
          >
            Cancel
          </Button>
          <Button
            variant="danger"
            size="pill"
            disabled={wiping || typed.trim() !== CONFIRM_WORD}
            onClick={deleteEverything}
          >
            {wiping ? <Spinner size="sm" /> : null}
            Delete everything
          </Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
