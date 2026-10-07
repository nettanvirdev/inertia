import * as React from "react";
import { Cpu, Plus } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { PREF, usePersistentState } from "@/lib/persist";
import { COMPUTER_STATUS_META, PROVIDER_META } from "@/data/computers";
import { Button } from "@/components/ui/button";
import { SearchInput, Input } from "@/components/ui/input";
import { Segmented } from "@/components/ui/segmented";
import { Spinner } from "@/components/ui/spinner";
import { ScrollArea } from "@/components/ui/scroll-area";
import { AvatarGroup } from "@/components/ui/avatar";
import { EmptyState } from "@/components/ui/empty-state";
import { Select } from "@/components/ui/select";
import { RadioGroup, RadioItem } from "@/components/ui/radio";
import {
  Dialog,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogFooter,
} from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { computers as machines } from "@/lib/computers";
import { ComputerDetail } from "./ComputerDetail";
import { GUTTER, HEADER_GAP } from "@/components/layout/View";

const STATE_OPTIONS = [
  { value: "all", label: "All" },
  { value: "running", label: "Running" },
  { value: "stopped", label: "Off" },
];

/** The dot is the whole status affordance in the list - colour plus nothing else. */
const DOT = {
  running: "bg-success",
  paused: "bg-warning",
  stopped: "bg-muted-foreground/40",
  error: "bg-destructive",
  missing: "bg-destructive",
};

/**
 * The sizes on offer.
 *
 * Named rather than three number fields, because "how many vCPU" is not a
 * question most people want asked - and because on Docker these become `--cpus`
 * and `--memory`, which are real limits with real consequences for a build.
 */
const SIZE_OPTIONS = [
  { value: "small", label: "Small", description: "2 vCPU · 4 GB · 20 GB disk" },
  { value: "medium", label: "Medium", description: "4 vCPU · 8 GB · 40 GB disk" },
  { value: "large", label: "Large", description: "8 vCPU · 16 GB · 80 GB disk" },
];

const SPECS = {
  small: { cpu: 2, memoryGb: 4, diskGb: 20 },
  medium: { cpu: 4, memoryGb: 8, diskGb: 40 },
  large: { cpu: 8, memoryGb: 16, diskGb: 80 },
};

export function ComputersView() {
  const { computers, agents, activeComputerId, setActiveComputerId, createComputer } = useApp();
  const { toast } = useToast();

  const [query, setQuery] = React.useState("");
  const [state, setState] = usePersistentState(PREF.computersKind, "all", (v) =>
    STATE_OPTIONS.some((o) => o.value === v)
  );
  const [creating, setCreating] = React.useState(false);

  const agentsById = React.useMemo(
    () => Object.fromEntries(agents.map((agent) => [agent.id, agent])),
    [agents]
  );

  const counts = React.useMemo(() => {
    const out = {};
    for (const computer of computers) out[computer.status] = (out[computer.status] ?? 0) + 1;
    return out;
  }, [computers]);

  const visible = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    return computers.filter((computer) => {
      if (state === "running" && computer.status !== "running") return false;
      if (state === "stopped" && computer.status === "running") return false;
      if (!q) return true;
      return [computer.name, computer.provider, computer.os].join(" ").toLowerCase().includes(q);
    });
  }, [computers, state, query]);

  const active = computers.find((computer) => computer.id === activeComputerId) ?? null;

  return (
    <>
      <header className={cn("flex h-14 shrink-0 items-center", HEADER_GAP, GUTTER)}>
        <h1 className="text-sm font-medium text-foreground">Computers</h1>
        <p className="hidden shrink-0 items-center gap-1.5 text-[11px] text-muted-foreground sm:flex">
          {Object.keys(COMPUTER_STATUS_META)
            .filter((status) => counts[status])
            .map((status) => (
              <span key={status} className="flex items-center gap-1">
                <span
                  aria-hidden="true"
                  className={cn(
                    "size-1.5 rounded-full",
                    status === "provisioning" ? "bg-info" : DOT[status]
                  )}
                />
                {counts[status]} {COMPUTER_STATUS_META[status].label.toLowerCase()}
              </span>
            ))}
        </p>

        <div className="ml-auto flex items-center gap-2">
          <SearchInput
            size="sm"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onClear={() => setQuery("")}
            placeholder="Search machines"
            className="w-44 lg:w-56"
            aria-label="Search computers"
          />
          <Segmented
            size="xs"
            value={state}
            onChange={setState}
            options={STATE_OPTIONS}
            label="Machine state"
          />
          <Button variant="primary" size="sm" onClick={() => setCreating(true)}>
            <Plus />
            New computer
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        {/*
          No rail at all when there are no machines.
          
          It used to render an empty 260px column holding the words "No machines
          yet." beside a full-height empty state saying the same thing at more
          length - two answers to one question, and the second one centred in
          what the first had left over, so the whole screen read as slightly
          off-centre. With nothing to list there is nothing to select, and the
          empty state can have the window.
        */}
        {computers.length === 0 ? null : (
          <ScrollArea className="w-[260px] shrink-0 px-2 pb-4">
            {visible.length === 0 ? (
              <p className="px-2 py-6 text-center text-[13px] text-muted-foreground">
                No machines match.
              </p>
            ) : (
              <ul className="flex flex-col gap-0.5">
                {visible.map((computer) => (
                  <li key={computer.id}>
                    <ComputerRow
                      computer={computer}
                      agents={(computer.assignedAgentIds ?? [])
                        .map((id) => agentsById[id])
                        .filter(Boolean)}
                      active={computer.id === activeComputerId}
                      onSelect={() => setActiveComputerId(computer.id)}
                    />
                  </li>
                ))}
              </ul>
            )}
          </ScrollArea>
        )}

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {active ? (
            <ComputerDetail computerId={active.id} />
          ) : (
            <EmptyState
              icon={Cpu}
              title={computers.length ? "No machine selected" : "No computers yet"}
              description={
                computers.length
                  ? "Pick a machine on the left to watch its screen, drive its terminal, or browse its files."
                  : "A computer is a machine an agent can drive - a Docker container here, or a sandbox in the cloud. Make one and assign a teammate to it."
              }
              className="h-full"
              action={
                computers.length ? null : (
                  <Button variant="primary" size="sm" onClick={() => setCreating(true)}>
                    <Plus />
                    New computer
                  </Button>
                )
              }
            />
          )}
        </div>
      </div>

      <NewComputerDialog
        open={creating}
        onOpenChange={setCreating}
        onCreate={async (draft) => {
          setCreating(false);
          toast({
            title: "Provisioning…",
            description:
              draft.provider === "docker"
                ? "The first Docker machine builds the sandbox image, which takes a few minutes."
                : `${draft.name} is being created.`,
          });
          try {
            await createComputer({
              name: draft.name,
              provider: draft.provider,
              // Daytona rejects a request that carries both a snapshot and a
              // size, so the snapshot is sent alone and the size is dropped.
              ...(draft.provider === "daytona"
                ? { image: draft.image }
                : { specs: SPECS[draft.size] }),
            });
            toast({ variant: "success", title: `${draft.name} is ready` });
          } catch (error) {
            toast({
              variant: "error",
              title: "Could not create it",
              description: error?.message ?? String(error),
            });
          }
        }}
      />
    </>
  );
}

function ComputerRow({ computer, agents, active, onSelect }) {
  const provider = PROVIDER_META[computer.provider];

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={active ? "true" : undefined}
      className={cn(
        "flex w-full items-center gap-2.5 rounded-xl px-2.5 py-2 text-left outline-none",
        "transition-colors duration-150 ease-out",
        "focus-visible:fill-nav",
        active ? "fill-nav-active" : "hover:fill-nav"
      )}
    >
      <span className="grid size-4 shrink-0 place-items-center">
        {computer.status === "provisioning" ? (
          <Spinner size="sm" label="Provisioning" />
        ) : (
          <span
            aria-hidden="true"
            className={cn("size-2 rounded-full", DOT[computer.status] ?? DOT.stopped)}
          />
        )}
      </span>

      <span className="flex min-w-0 flex-1 flex-col">
        <span
          className={cn(
            "truncate text-[13px]",
            active ? "font-medium text-foreground" : "text-foreground"
          )}
          title={computer.name}
        >
          {computer.name}
        </span>
        <span className="truncate text-[11px] text-muted-foreground">
          {provider?.label ?? computer.provider}
          {computer.os ? ` · ${computer.os}` : ""}
        </span>
      </span>

      {agents.length ? (
        <AvatarGroup size="xs" max={3} className="shrink-0">
          {agents.map((agent) => (
            <AgentAvatar key={agent.id} agent={agent} size="xs" />
          ))}
        </AvatarGroup>
      ) : null}
    </button>
  );
}

/**
 * Making a machine.
 *
 * The providers are asked rather than listed, because whether one can run is
 * not a static fact: Docker may not be installed, its daemon may be stopped,
 * Daytona may have no key. A picker that offered all three regardless would let
 * someone choose a machine that cannot be made and find out a minute later.
 */
function NewComputerDialog({ open, onOpenChange, onCreate }) {
  const [name, setName] = React.useState("");
  const [provider, setProvider] = React.useState("docker");
  const [size, setSize] = React.useState("medium");
  const [providers, setProviders] = React.useState(null);
  /*
    Daytona does not take a size.
    
    It refuses one outright - "Cannot specify Sandbox resources when using a
    snapshot" - because on Daytona the snapshot IS the size: `daytona-medium`
    means 2 vCPU and 4 GB, and there is no way to ask for 4 vCPU except by
    naming a snapshot that has them. So the Small/Medium/Large control is a lie
    on that provider, and the honest version is a list of what the account
    actually has. The user picked Medium and got 1 vCPU; that is the bug this
    replaces.
  */
  const [catalogue, setCatalogue] = React.useState(null);
  const [snapshot, setSnapshot] = React.useState("");

  React.useEffect(() => {
    if (!open) return;
    setName("");
    setSize("medium");
    setProviders(null);
    machines
      .providers()
      .then((rows) => {
        setProviders(rows);
        const usable = rows.find((row) => row.ok);
        if (usable) setProvider(usable.id);
      })
      .catch(() => setProviders([]));
  }, [open]);

  // Fetched when Daytona is chosen rather than when the dialog opens: it is a
  // network call to someone else's API, and a user making a Docker machine
  // should not wait on it or pay for it.
  React.useEffect(() => {
    if (!open || provider !== "daytona") return;
    let live = true;
    setCatalogue(null);
    machines
      .catalogue("daytona")
      .then((rows) => {
        if (!live) return;
        setCatalogue(rows);
        // Default to the machine that has the desktop if the account has one,
        // and otherwise to the smallest, which is the cheapest way to find out
        // whether any of this works.
        const best = rows.find((row) => row.desktop) ?? rows[0];
        setSnapshot(best?.name ?? "");
      })
      .catch(() => live && setCatalogue([]));
    return () => {
      live = false;
    };
  }, [open, provider]);

  const snapshotOptions = (catalogue ?? []).map((row) => ({
    value: row.name,
    label: row.name,
    description:
      [
        row.cpu ? `${row.cpu} vCPU` : null,
        row.memoryGb ? `${row.memoryGb} GB` : null,
        row.diskGb ? `${row.diskGb} GB disk` : null,
      ]
        .filter(Boolean)
        .join(" · ") + (row.desktop ? " · desktop and browser" : ""),
  }));

  const trimmed = name.trim();
  const chosen = providers?.find((row) => row.id === provider) ?? null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange} size="md">
      <DialogTitle>New computer</DialogTitle>
      <DialogDescription>
        A machine an agent can drive. Assign teammates to it once it is up.
      </DialogDescription>

      <DialogBody className="flex flex-col gap-4">
        <Field label="Name">
          <Input
            size="sm"
            value={name}
            autoFocus
            onChange={(event) => setName(event.target.value)}
            placeholder="Build box"
            aria-label="Computer name"
          />
        </Field>

        <Field label="Where it runs">
          {providers === null ? (
            <div className="flex h-9 items-center gap-2 text-[12px] text-muted-foreground">
              <Spinner size="sm" />
              Checking what is available…
            </div>
          ) : (
            <RadioGroup value={provider} onChange={setProvider} label="Provider">
              {(providers ?? []).map((row) => (
                <RadioItem
                  key={row.id}
                  value={row.id}
                  disabled={!row.ok}
                  label={row.label}
                  description={
                    row.ok
                      ? row.warning || row.blurb
                      : [row.reason, row.hint].filter(Boolean).join(" ")
                  }
                />
              ))}
            </RadioGroup>
          )}
        </Field>

        {/* Keyed on which control the provider wants, so the size picker and
            the snapshot list swap with a fade rather than one becoming the
            other under the cursor. */}
        <div key={provider === "daytona" ? "snapshot" : "size"} className="animate-fade-in">
          {provider === "daytona" ? (
            <Field label="Machine type">
              {catalogue === null ? (
                <div className="flex h-9 items-center gap-2 text-[12px] text-muted-foreground">
                  <Spinner size="sm" />
                  Reading your Daytona snapshots…
                </div>
              ) : (
                <>
                  <Select
                    size="md"
                    value={snapshot}
                    onChange={setSnapshot}
                    options={snapshotOptions}
                    ariaLabel="Machine type"
                    emptyLabel="No snapshots on this account"
                  />
                  <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">
                    {snapshotOptions.length === 0
                      ? "Daytona could not list any snapshots. Check the key under Settings, Secrets."
                      : (catalogue ?? []).some((row) => row.desktop)
                        ? "A snapshot carries its own size and its own image, so this is both."
                        : "None of these carry the Inertia sandbox image, so this machine will have a shell and files but no screen or browser. Settings, Computers shows how to push it."}
                  </p>
                </>
              )}
            </Field>
          ) : (
            <Field label="Size">
              <Select
                size="md"
                value={size}
                onChange={setSize}
                options={SIZE_OPTIONS}
                ariaLabel="Size"
              />
            </Field>
          )}
        </div>

        {chosen?.warning ? (
          <p className="animate-fade-in rounded-xl fill-whisper px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
            {chosen.warning}
          </p>
        ) : null}
      </DialogBody>

      <DialogFooter>
        <Button
          variant="secondary"
          size="pill"
          className="w-full sm:w-auto"
          onClick={() => onOpenChange(false)}
        >
          Cancel
        </Button>
        <Button
          variant="primary"
          size="pill"
          className="w-full sm:w-auto"
          disabled={!trimmed || !chosen?.ok || (provider === "daytona" && !snapshot)}
          onClick={() => onCreate({ name: trimmed, provider, size, image: snapshot })}
        >
          Create
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

function Field({ label, children }) {
  return (
    // a <label> would forward its click into the Select's trigger and reopen it
    <div className="flex flex-col gap-1.5">
      <span className="text-xs text-foreground/90">{label}</span>
      {children}
    </div>
  );
}
