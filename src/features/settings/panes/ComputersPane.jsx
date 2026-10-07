import * as React from "react";
import { useApp } from "@/lib/store";
import { Hammer, Icon, KeyRound } from "@/components/icons";
import { PROVIDER_META } from "@/data/computers";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { useToast } from "@/components/ui/toast";
import { computers as machines } from "@/lib/computers";
import { useCached } from "@/lib/cache";
import { SettingsCard, SettingsRow, SettingsSection } from "../SettingsRow";

/**
 * The computers settings.
 *
 * Every control here used to be `useState` that was thrown away when the sheet
 * closed, over a hardcoded list of four providers with invented "connected"
 * badges - one of which named a backend the app has never had. All of it is
 * real now: the settings are a document in the workspace, and the provider list
 * is the backend being asked whether each one can actually run.
 *
 * The two policy switches that were here - "allow agents to install packages",
 * "allow network egress" - are gone rather than wired up. Neither was
 * enforceable: nothing read them, and enforcing them would mean a seccomp
 * profile and a network policy that this app does not have. What an agent may
 * do on its machine is its permission ruleset, which is real, and pointing at
 * that is more honest than a switch that stops nothing.
 */

const IDLE_OPTIONS = [
  {
    value: "0",
    label: "Never",
    description: "A cloud sandbox keeps billing until stopped",
  },
  { value: "5", label: "After 5 minutes" },
  { value: "15", label: "After 15 minutes" },
  { value: "30", label: "After 30 minutes" },
  { value: "120", label: "After 2 hours" },
];

const SIZE_OPTIONS = [
  { value: "2-4-20", label: "Small", description: "2 vCPU · 4 GB · 20 GB" },
  { value: "4-8-40", label: "Medium", description: "4 vCPU · 8 GB · 40 GB" },
  { value: "8-16-80", label: "Large", description: "8 vCPU · 16 GB · 80 GB" },
];

const specKey = (specs) => `${specs?.cpu ?? 4}-${specs?.memoryGb ?? 8}-${specs?.diskGb ?? 40}`;

/**
 * What you can actually do about a provider, from here.
 *
 * This used to be one button per row that opened a toast saying the real
 * settings were "edited in the provider console" - a sentence about a console
 * this app does not have, on a row for a backend it had never implemented. Each
 * row now offers the one thing that would genuinely change its state, and rows
 * with nothing to offer say so instead of offering a button.
 */
function ProviderAction({ provider, building, onRecheck, onBuild, onAddKey }) {
  if (provider.id === "daytona" && !provider.ok) {
    // The key is a workspace secret, and Secrets is where secrets are pasted -
    // so this goes there rather than growing a second field for the same value.
    return (
      <Button variant="subtle" size="xs" onClick={onAddKey}>
        <KeyRound />
        Add key
      </Button>
    );
  }

  if (provider.id === "docker" && provider.ok) {
    return (
      <Button variant="subtle" size="xs" disabled={building} onClick={onBuild}>
        <Hammer />
        {building ? "Building…" : "Build image"}
      </Button>
    );
  }

  if (provider.id === "local") {
    return <span className="text-[0.6875rem] text-muted-foreground">Nothing to set up</span>;
  }

  return (
    <Button variant="subtle" size="xs" onClick={onRecheck}>
      Re-check
    </Button>
  );
}

function specsFrom(key) {
  const [cpu, memoryGb, diskGb] = String(key).split("-").map(Number);
  return { cpu, memoryGb, diskGb };
}

export function ComputersPane() {
  const { computers, user, setPreference, setSettingsTab } = useApp();
  const { toast } = useToast();

  /**
   * Everything this pane reads, cached for the whole session.
   *
   * Three IPC calls that used to go out on every mount, behind a spinner that
   * covered the pane while they did. Leaving the tab unmounted this component
   * and threw the answers away, so coming back paid for all three again -
   * which is what made a settings tab feel slower than the chat it was opened
   * from. Now the first visit fetches, and every visit after it renders from
   * what is remembered and re-asks quietly only when the answer is old.
   *
   * One key rather than three: they are read together, shown together, and a
   * pane that had two of the three would have to render a third state that
   * does not exist.
   */
  const {
    data,
    loading,
    refresh,
    set: remember,
  } = useCached("computers.settings", async () => {
    const [stored, rows, img] = await Promise.all([
      machines.settings(),
      machines.providers(),
      machines.image().catch(() => null),
    ]);
    return { settings: stored, providers: rows, image: img };
  });

  // `null` is a real answer here - no desktop bridge, nothing to configure -
  // and it is what the pane below tests for. `undefined` is "not asked yet".
  const settings = data === undefined ? null : (data?.settings ?? null);
  const providers = data?.providers ?? null;
  const image = data?.image ?? null;

  const [apiUrl, setApiUrl] = React.useState("");
  const [daytonaImage, setDaytonaImage] = React.useState("");
  const [building, setBuilding] = React.useState(false);
  const [buildLog, setBuildLog] = React.useState("");

  /**
   * The two text fields are seeded from the settings once and then left alone.
   *
   * They save on blur, so they hold what the person is typing. A background
   * refresh landing mid-sentence must not overwrite that, which is why this
   * keys off the pane being empty rather than off the settings changing.
   */
  const seeded = React.useRef(false);
  React.useEffect(() => {
    if (seeded.current || !settings) return;
    seeded.current = true;
    setApiUrl(settings.daytonaApiUrl ?? "");
    setDaytonaImage(settings.daytonaImage ?? "");
  }, [settings]);

  /** Saved on change, like every other setting in this app. */
  async function save(patch) {
    const held = data ?? { settings: {}, providers: null, image: null };
    remember({ ...held, settings: { ...held.settings, ...patch } });
    try {
      const next = await machines.saveSettings(patch);
      remember({ ...held, settings: next });
    } catch (error) {
      toast({ variant: "error", title: "Could not save", description: error?.message });
      refresh();
    }
  }

  /**
   * Build the image now rather than in the middle of making a machine.
   *
   * The first Docker computer builds it either way. Doing it here, where the
   * output is on screen, is the difference between a fifteen-minute build and
   * fifteen minutes of a dialog that looks hung.
   */
  async function build() {
    setBuilding(true);
    setBuildLog("");
    const off = machines.onEvent((event) => {
      if (event.type === "output") setBuildLog((prev) => (prev + event.text).slice(-4000));
    });
    try {
      const built = await machines.buildImage();
      toast({
        variant: "success",
        title: built.built ? "Image built" : "Image was already built",
        description: built.ref,
      });
      refresh();
    } catch (error) {
      toast({
        variant: "error",
        title: "The build failed",
        description: error?.message,
      });
    } finally {
      off?.();
      setBuilding(false);
    }
  }

  // Only the very first visit, when there is genuinely nothing to show. Every
  // later one renders from the cache, which is the whole point of it.
  if (loading) {
    return (
      <div className="flex w-full justify-center py-10">
        <Spinner />
      </div>
    );
  }

  if (!settings) {
    return (
      <p className="text-xs text-muted-foreground">
        Computers need the desktop app and a workspace folder.
      </p>
    );
  }

  return (
    <div className="w-full">
      <SettingsSection title="Defaults">
        <SettingsRow
          label="Default computer"
          description="Offered first when assigning a machine to a new agent."
          control={
            <Select
              size="xs"
              className="w-56"
              panelClassName="w-64"
              ariaLabel="Default computer"
              value={user.preferences.defaultComputerId ?? ""}
              onChange={(value) => setPreference("defaultComputerId", value || null)}
              options={[
                { value: "", label: "None" },
                ...computers.map((computer) => ({
                  value: computer.id,
                  label: computer.name,
                  description: `${PROVIDER_META[computer.provider]?.label ?? computer.provider} · ${computer.status}`,
                })),
              ]}
            />
          }
        />
        <SettingsRow
          label="Default provider"
          description="Where a new computer is made unless you pick otherwise."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Default provider"
              value={settings.defaultProvider}
              onChange={(value) => save({ defaultProvider: value })}
              options={(providers ?? []).map((provider) => ({
                value: provider.id,
                label: provider.label,
                disabled: !provider.ok,
                description: provider.ok ? provider.blurb : provider.reason,
              }))}
            />
          }
        />
        <SettingsRow
          label="Default size"
          description={
            "On Docker these become real limits: --cpus and --memory on the container. " +
            "Daytona ignores them - it refuses a size alongside an image, so a cloud sandbox " +
            "gets whatever its snapshot was registered with."
          }
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Default size"
              value={specKey(settings.defaultSpecs)}
              onChange={(value) => save({ defaultSpecs: specsFrom(value) })}
              options={SIZE_OPTIONS}
            />
          }
        />
      </SettingsSection>

      <SettingsSection title="Lifecycle">
        <SettingsRow
          label="Stop a cloud sandbox after idle"
          description="Daytona bills for a running sandbox whether anything is using it or not. Docker and local machines ignore this - nothing is metered."
          control={
            <Select
              size="xs"
              className="w-56"
              ariaLabel="Stop after idle"
              value={String(settings.autoStopMinutes ?? 30)}
              onChange={(value) => save({ autoStopMinutes: Number(value) })}
              options={IDLE_OPTIONS}
            />
          }
        />
        <SettingsRow
          label="Refresh the meters every"
          description="How often the computers screen asks each machine what it is doing. Only while that screen is open."
          control={
            <Select
              size="xs"
              className="w-40"
              ariaLabel="Refresh interval"
              value={String(settings.pollSeconds ?? 10)}
              onChange={(value) => save({ pollSeconds: Number(value) })}
              options={[
                { value: "5", label: "5 seconds" },
                { value: "10", label: "10 seconds" },
                { value: "30", label: "30 seconds" },
                { value: "60", label: "1 minute" },
              ]}
            />
          }
        />
      </SettingsSection>

      <SettingsSection title="Daytona">
        <SettingsRow
          label="Image"
          description={
            "What Daytona pulls for a new sandbox. Leave empty and it uses a plain Debian, " +
            "which can run commands but has no display and no browser. Push the image below " +
            "to a registry Daytona can reach and name it here to get the whole machine."
          }
          control={
            <Input
              size="xs"
              className="w-64"
              value={daytonaImage}
              placeholder="debian:12-slim"
              aria-label="Daytona image"
              onChange={(event) => setDaytonaImage(event.target.value)}
              onBlur={() => save({ daytonaImage: daytonaImage.trim() })}
            />
          }
        />
        <SettingsRow
          label="API endpoint"
          description="Leave empty for Daytona's own cloud. Set it to point at a self-hosted install. The key itself is a workspace secret called DAYTONA_API_KEY, under Secrets."
          control={
            <div className="flex items-center gap-2">
              <Input
                size="xs"
                className="w-64"
                value={apiUrl}
                placeholder="https://app.daytona.io/api"
                aria-label="Daytona API endpoint"
                onChange={(event) => setApiUrl(event.target.value)}
                onBlur={() => save({ daytonaApiUrl: apiUrl.trim() })}
              />
            </div>
          }
        />
      </SettingsSection>

      <SettingsSection flat title="Providers">
        {/* one provider per line, full width: name and status on the left, the
            action on the right - the same left/right rhythm as every other row */}
        <SettingsCard className="flex flex-col gap-1 py-1">
          {(providers ?? []).map((provider) => {
            const meta = PROVIDER_META[provider.id];
            return (
              <div key={provider.id} className="flex min-w-0 items-start gap-2.5 py-1.5">
                <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full fill-control text-muted-foreground">
                  <Icon name={meta?.icon ?? "Container"} className="size-3.5" aria-hidden="true" />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-xs text-foreground/90">{provider.label}</span>
                    <Badge variant={provider.ok ? "success" : "neutral"} size="sm">
                      {provider.ok ? "Ready" : "Unavailable"}
                    </Badge>
                  </div>
                  <p className="mt-0.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
                    {provider.ok
                      ? provider.warning || provider.blurb
                      : [provider.reason, provider.hint].filter(Boolean).join(" ")}
                  </p>
                </div>
                <ProviderAction
                  provider={provider}
                  building={building}
                  // Recheck is the one place that must not be served from the
                  // cache: it is the button you press because you just
                  // installed Docker.
                  onRecheck={refresh}
                  onBuild={build}
                  onAddKey={() => setSettingsTab("secrets")}
                />
              </div>
            );
          })}
        </SettingsCard>
      </SettingsSection>

      <SettingsSection flat title="The sandbox image">
        <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
          Docker and Daytona machines are built from one image, defined in the app's{" "}
          <span className="font-mono">sandbox/</span> folder: Debian, the tools an agent would
          otherwise ask you to install, and an X server so its desktop and browser panes show
          something real. Editing it and bumping the tag in{" "}
          <span className="font-mono">image.json</span> changes what a new machine gets. A machine
          that already exists keeps the image it was made from.
        </p>

        {image ? (
          <SettingsCard className="flex flex-col gap-2">
            <div className="flex min-w-0 items-center gap-2">
              <span className="min-w-0 flex-1 truncate font-mono text-[0.6875rem] text-foreground/90">
                {image.ref}
              </span>
              <Badge variant={image.buildable ? "neutral" : "warning"} size="sm">
                {image.buildable ? "Buildable" : "No Dockerfile found"}
              </Badge>
            </div>
            <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
              Building it now takes a couple of minutes and pulls a few hundred megabytes. Doing it
              here rather than when you make your first machine means the wait is one you chose.
              Docker machines use it straight away.
            </p>
            <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
              Daytona cannot see an image that only exists here. To use it in the cloud, push it to
              a registry and put the pushed name in the Daytona Image box above:
            </p>
            <pre className="overflow-x-auto rounded-xl fill-whisper p-2 font-mono text-[0.625rem] leading-relaxed text-muted-foreground">
              {`docker tag ${image.ref} <you>/inertia-sandbox:0.1.0
docker push <you>/inertia-sandbox:0.1.0`}
            </pre>
            {buildLog ? (
              <pre className="max-h-40 animate-fade-in overflow-auto rounded-xl fill-whisper p-2 font-mono text-[0.625rem] leading-relaxed text-muted-foreground">
                {buildLog}
              </pre>
            ) : null}
          </SettingsCard>
        ) : null}
      </SettingsSection>
    </div>
  );
}
