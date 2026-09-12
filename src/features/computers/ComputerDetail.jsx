import * as React from "react";
import {
  Camera,
  Cloud,
  Folder,
  Globe,
  Icon,
  MonitorPlay,
  MoreHorizontal,
  Pause,
  Play,
  RotateCcw,
  RotateCw,
  Square,
  Terminal,
  Trash2,
  Users,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { PREF, usePersistentState } from "@/lib/persist";
import { PROVIDER_META, COMPUTER_STATUS_META } from "@/data/computers";
import { formatBytes, relativeTime } from "@/data";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Badge } from "@/components/ui/badge";
import { Meter } from "@/components/ui/progress";
import { Tabs, TabPanel } from "@/components/ui/tabs";
import { ScrollArea } from "@/components/ui/scroll-area";
import { DropdownMenu, MenuItem, MenuSeparator } from "@/components/ui/dropdown-menu";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { computers as machines } from "@/lib/computers";
import { DesktopPane } from "./DesktopPane";
import { TerminalPane } from "./TerminalPane";
import { FilesPane } from "./FilesPane";
import { BrowserPane } from "./BrowserPane";
import { SnapshotsDialog } from "./SnapshotsDialog";
import { AssignAgentsDialog } from "./AssignAgentsDialog";
import { ImportCookiesDialog, forComputer } from "./ImportCookiesDialog";

const STATUS_VARIANT = {
  running: "success",
  paused: "warning",
  stopped: "neutral",
  provisioning: "info",
  error: "danger",
  missing: "danger",
};

const TABS = [
  { value: "desktop", label: "Desktop", icon: MonitorPlay },
  { value: "terminal", label: "Terminal", icon: Terminal },
  { value: "files", label: "Files", icon: Folder },
  { value: "browser", label: "Browser", icon: Globe },
];

const GB = 1024 ** 3;

export function ComputerDetail({ computerId }) {
  const { computers, runComputerAction, deleteComputer, agents } = useApp();
  const { toast } = useToast();

  const [tab, setTab] = usePersistentState(PREF.computersTab, "desktop", (v) =>
    TABS.some((t) => t.value === v)
  );
  const [confirmStop, setConfirmStop] = React.useState(false);
  const [confirmDestroy, setConfirmDestroy] = React.useState(false);
  const [snapshotsOpen, setSnapshotsOpen] = React.useState(false);
  const [assignOpen, setAssignOpen] = React.useState(false);
  const [cookiesOpen, setCookiesOpen] = React.useState(false);
  const [working, setWorking] = React.useState(false);

  const computer = computers.find((c) => c.id === computerId);
  if (!computer) return null;

  const status = computer.status;
  const provider = PROVIDER_META[computer.provider];
  const statusMeta = COMPUTER_STATUS_META[status];
  const busy = status === "provisioning" || working;
  const assigned = computer.assignedAgentIds ?? [];

  /**
   * Every lifecycle button, awaited.
   *
   * These used to rewrite a status string and return. Starting a container
   * takes a second and starting a Daytona sandbox can take thirty, so the
   * button has to stay busy until the provider has actually answered - and has
   * to be able to say that it failed, which "set the status to running" never
   * could.
   */
  async function act(verb, success) {
    setWorking(true);
    try {
      const updated = await runComputerAction(computer.id, verb);
      if (success) toast({ variant: "success", title: success(updated) });
    } catch (error) {
      toast({
        variant: "error",
        title: `Could not ${verb} ${computer.name}`,
        description: error?.message ?? String(error),
      });
    } finally {
      setWorking(false);
    }
  }

  const start = () => act(status === "paused" ? "resume" : "start");

  return (
    <div className="flex h-full min-h-0 flex-col px-5 pb-6">
      <div className="flex shrink-0 flex-wrap items-start gap-x-4 gap-y-3 pt-1 pb-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2.5">
            <h2 className="truncate text-[20px] font-medium text-foreground" title={computer.name}>
              {computer.name}
            </h2>
            <Badge variant={STATUS_VARIANT[status] ?? "neutral"} dot>
              {statusMeta?.label ?? status}
            </Badge>
          </div>

          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            <Chip icon={provider?.icon}>{provider?.label ?? computer.provider}</Chip>
            {computer.os ? <Chip>{computer.os}</Chip> : null}
            {computer.image ? <Chip>{computer.image}</Chip> : null}
            <span className="pl-1 text-[11px] text-muted-foreground">
              {status === "running" && computer.startedAt
                ? `up since ${relativeTime(computer.startedAt)}`
                : `not running`}
            </span>
          </div>

          {computer.error ? (
            <p className="mt-2 animate-fade-in text-[12px] text-destructive-ink">{computer.error}</p>
          ) : null}
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          <Button
            size="sm"
            variant="subtle"
            disabled={busy || status === "running"}
            onClick={start}
          >
            <Play />
            Start
          </Button>
          <Button
            size="sm"
            variant="subtle"
            // Only Docker can pause: a paused container keeps its process tree,
            // which is what makes it different from stopping. The other two have
            // no such state, and offering a button that answers "cannot" is
            // worse than not offering it.
            disabled={busy || status !== "running" || computer.provider !== "docker"}
            title={
              computer.provider === "docker" ? undefined : "Only Docker machines can be paused."
            }
            onClick={() => act("pause")}
          >
            <Pause />
            Pause
          </Button>
          <Button
            size="sm"
            variant="subtle"
            disabled={busy || status === "stopped" || status === "missing"}
            onClick={() => setConfirmStop(true)}
          >
            <Square />
            Stop
          </Button>
          <Button
            size="sm"
            variant="subtle"
            disabled={busy || status === "missing"}
            onClick={async () => {
              await act("stop");
              await act("start", () => `${computer.name} is back up`);
            }}
          >
            <RotateCw />
            Restart
          </Button>

          <DropdownMenu
            align="end"
            ariaLabel="Machine actions"
            trigger={
              <IconButton size="lg" label="More machine actions">
                <MoreHorizontal />
              </IconButton>
            }
          >
            <MenuItem
              icon={Camera}
              disabled={status !== "running"}
              onSelect={async () => {
                setWorking(true);
                try {
                  const snap = await machines.snapshot(computer.id);
                  toast({
                    variant: "success",
                    title: "Snapshot taken",
                    description: `${computer.name} · ${snap.name}`,
                  });
                } catch (error) {
                  toast({ variant: "error", title: "Snapshot failed", description: error?.message });
                } finally {
                  setWorking(false);
                }
              }}
            >
              Take a snapshot
            </MenuItem>
            <MenuItem icon={RotateCcw} onSelect={() => setSnapshotsOpen(true)}>
              Snapshots and restore
            </MenuItem>
            <MenuItem icon={Users} onSelect={() => setAssignOpen(true)}>
              Assign agents
            </MenuItem>
            <MenuItem
              icon={Cloud}
              disabled={status !== "running"}
              onSelect={() => setCookiesOpen(true)}
            >
              Bring your sessions
            </MenuItem>
            <MenuSeparator />
            <MenuItem icon={Trash2} danger onSelect={() => setConfirmDestroy(true)}>
              Destroy
            </MenuItem>
          </DropdownMenu>
        </div>
      </div>

      <div className="grid shrink-0 grid-cols-2 gap-2 lg:grid-cols-5">
        <ResourceCell>
          <Meter label="CPU" value={computer.usage?.cpuPct ?? 0} tone={tone(computer.usage?.cpuPct)} />
          <Spec>{measured(computer.usage?.cpuPct, `${computer.specs?.cpu || "?"} vCPU`)}</Spec>
        </ResourceCell>
        <ResourceCell>
          <Meter label="Memory" value={computer.usage?.memPct ?? 0} tone={tone(computer.usage?.memPct)} />
          <Spec>
            {measured(
              computer.usage?.memPct,
              computer.specs?.memoryGb ? formatBytes(computer.specs.memoryGb * GB) : "no limit"
            )}
          </Spec>
        </ResourceCell>
        <ResourceCell>
          <Meter label="Disk" value={computer.usage?.diskPct ?? 0} tone={tone(computer.usage?.diskPct)} />
          <Spec>
            {measured(
              computer.usage?.diskPct,
              computer.specs?.diskGb ? formatBytes(computer.specs.diskGb * GB) : "no quota"
            )}
          </Spec>
        </ResourceCell>
        <ResourceCell>
          <StatLabel>Snapshots</StatLabel>
          <p className="text-[15px] tabular-nums text-foreground">{computer.snapshotCount ?? 0}</p>
          <Spec>{computer.snapshotCount ? "restorable" : "none yet"}</Spec>
        </ResourceCell>
        <ResourceCell>
          <StatLabel>Last used</StatLabel>
          <p className="text-[15px] text-foreground">{relativeTime(computer.lastUsedAt)}</p>
          <Spec>
            {assigned.length} agent{assigned.length === 1 ? "" : "s"} assigned
          </Spec>
        </ResourceCell>
      </div>

      <Tabs
        className="mt-5 shrink-0"
        variant="strip"
        value={tab}
        onChange={setTab}
        items={TABS}
        ariaLabel="Machine surfaces"
        idPrefix={`cmp-${computer.id}`}
      />

      {/*
        The tab body takes exactly what the window has left over: the bands above
        are shrink-0, this flexes. The ScrollArea is only a floor - it catches the
        overflow at window heights too short for the panes' own min-heights, so
        the page itself never grows a scrollbar.
      */}
      <ScrollArea className="mt-3 min-h-0 flex-1">
        <TabPanel
          value="desktop"
          activeValue={tab}
          idPrefix={`cmp-${computer.id}`}
          className="flex h-full min-h-0 animate-fade-in flex-col"
        >
          <DesktopPane key={computer.id} computer={computer} onStart={start} />
        </TabPanel>
        <TabPanel
          value="terminal"
          activeValue={tab}
          idPrefix={`cmp-${computer.id}`}
          className="flex h-full min-h-0 animate-fade-in flex-col"
        >
          <TerminalPane key={computer.id} computer={computer} />
        </TabPanel>
        <TabPanel
          value="files"
          activeValue={tab}
          idPrefix={`cmp-${computer.id}`}
          className="flex h-full min-h-0 animate-fade-in flex-col"
        >
          <FilesPane key={computer.id} computer={computer} />
        </TabPanel>
        <TabPanel
          value="browser"
          activeValue={tab}
          idPrefix={`cmp-${computer.id}`}
          className="flex h-full min-h-0 animate-fade-in flex-col"
        >
          <BrowserPane key={computer.id} computer={computer} />
        </TabPanel>
      </ScrollArea>

      <SnapshotsDialog
        open={snapshotsOpen}
        onOpenChange={setSnapshotsOpen}
        computer={computer}
      />
      <AssignAgentsDialog
        open={assignOpen}
        onOpenChange={setAssignOpen}
        computer={computer}
        agents={agents}
      />
      <ImportCookiesDialog
        open={cookiesOpen}
        onOpenChange={setCookiesOpen}
        target={forComputer(computer)}
      />

      <ConfirmDialog
        open={confirmStop}
        onOpenChange={setConfirmStop}
        destructive
        title={`Stop ${computer.name}?`}
        description="Running processes are killed and anything not written to disk is lost. Snapshots and the machine's disk are kept."
        confirmLabel="Stop machine"
        onConfirm={() => {
          setConfirmStop(false);
          act("stop");
        }}
      />

      <ConfirmDialog
        open={confirmDestroy}
        onOpenChange={setConfirmDestroy}
        destructive
        title={`Destroy ${computer.name}?`}
        description={
          `This deletes the machine and its disk. ` +
          `${assigned.length} assigned ${assigned.length === 1 ? "agent loses" : "agents lose"} ` +
          `their computer. It cannot be undone.`
        }
        confirmLabel="Destroy"
        onConfirm={async () => {
          const name = computer.name;
          setConfirmDestroy(false);
          try {
            await deleteComputer(computer.id);
            toast({
              variant: "warning",
              title: `${name} destroyed`,
              description: "The machine and its disk are gone.",
            });
          } catch (error) {
            toast({ variant: "error", title: "Could not destroy it", description: error?.message });
          }
        }}
      />
    </div>
  );
}

function tone(pct) {
  if (pct == null) return "neutral";
  if (pct >= 85) return "danger";
  if (pct >= 65) return "warning";
  return "neutral";
}

/** A provider that cannot measure something answers null, and the caption says
 *  so rather than letting a 0% bar read as an idle machine. */
function measured(pct, fallback) {
  return pct == null ? "not measured" : fallback;
}

function ResourceCell({ children }) {
  return (
    // `bg-card/30 dark:bg-card` was a rung mixed by hand, and forked by mode
    // because one alpha could not serve both. The ladder has that step now.
    <div className="flex flex-col gap-1.5 rounded-2xl card-surface-subtle px-3 py-2.5">
      {children}
    </div>
  );
}

function StatLabel({ children }) {
  return <p className="text-[11px] text-muted-foreground">{children}</p>;
}

function Spec({ children }) {
  return <p className="text-[11px] text-muted-foreground">{children}</p>;
}

function Chip({ icon, children }) {
  return (
    <span
      className={cn(
        "inline-flex h-5 items-center gap-1 rounded-full fill-secondary px-2",
        "text-[11px] text-muted-foreground"
      )}
    >
      {icon ? <Icon name={icon} className="size-3" aria-hidden="true" /> : null}
      {children}
    </span>
  );
}
