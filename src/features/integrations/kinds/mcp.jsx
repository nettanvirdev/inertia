import * as React from "react";
import {
  CircleAlert,
  CircleCheck,
  Globe,
  Pencil,
  Play,
  PowerOff,
  SearchX,
  Server,
  Terminal,
  Trash2,
  Wrench,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { mcp } from "@/lib/integrations";
import { GUTTER } from "@/components/layout/View";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Segmented } from "@/components/ui/segmented";
import { ScrollArea } from "@/components/ui/scroll-area";
import { EmptyState } from "@/components/ui/empty-state";
import { IconButton } from "@/components/ui/icon-button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Disclosure,
  Field,
  ListEditor,
  Mono,
  SECRET_SYNTAX,
  Section,
  SecretKeyValueEditor,
  ToggleRow,
} from "../fields";

/**
 * MCP servers.
 *
 * The draft is the stored record, field for field, with no translation layer:
 * the backend validates `command`, `args`, `env`, `url` and `headers` by
 * those names, and a form that renamed them would make every error message it
 * sends back describe a field the user cannot see.
 *
 * Test is the most valuable control here. An MCP server that does not start is
 * otherwise completely opaque - the tools simply are not there, with nothing on
 * screen saying why - so proving it before it is saved is the difference
 * between a config that works and a config that looks right.
 */

const STATUS = {
  connected: { label: "Connected", variant: "success" },
  connecting: { label: "Starting", variant: "info" },
  disabled: { label: "Disabled", variant: "neutral" },
  failed: { label: "Failed", variant: "danger" },
  idle: { label: "Not connected", variant: "neutral" },
};

function statusOf(record) {
  return STATUS[record?.status] ?? { label: record?.status || "Unknown", variant: "neutral" };
}

const blank = () => ({
  name: "",
  type: "stdio",
  command: "",
  args: [],
  cwd: "",
  env: {},
  url: "",
  headers: {},
  timeoutMs: 0,
  enabled: true,
});

function toDraft(record) {
  return {
    ...blank(),
    ...record,
    type: record?.type === "http" ? "http" : "stdio",
    args: Array.isArray(record?.args) ? record.args : [],
    env: record?.env ?? {},
    headers: record?.headers ?? {},
  };
}

/** Only the configuration, never the status fields the backend owns. */
function toRecord(draft) {
  const base = {
    name: draft.name.trim() || "MCP server",
    type: draft.type,
    enabled: draft.enabled !== false,
    timeoutMs: Number(draft.timeoutMs) > 0 ? Number(draft.timeoutMs) : 0,
  };
  if (draft.type === "http") {
    return {
      ...base,
      url: draft.url.trim(),
      headers: draft.headers ?? {},
      command: "",
      args: [],
      cwd: "",
      env: {},
    };
  }
  return {
    ...base,
    command: draft.command.trim(),
    args: (draft.args ?? []).map((arg) => String(arg)).filter((arg) => arg.length > 0),
    cwd: draft.cwd?.trim() ?? "",
    env: draft.env ?? {},
    url: "",
    headers: {},
  };
}

function validate(draft) {
  if (!draft.name.trim()) return "Give the server a name.";
  if (draft.type === "stdio" && !draft.command.trim())
    return "A local server needs a command to run.";
  if (draft.type === "http" && !draft.url.trim()) return "A remote server needs a URL.";
  return null;
}

function describe(record) {
  if (record.type === "http") return record.url;
  return [record.command, ...(record.args ?? [])].filter(Boolean).join(" ");
}

/**
 * When the app will try this one again, in words.
 *
 * A failed server is being retried on a backoff whether or not anyone is
 * watching, and a row that does not say so reads as a dead end the user has to
 * do something about. Rounded up to the nearest sensible unit, because the exact
 * second is not information anybody acts on.
 */
function retryIn(record) {
  const at = Number(record?.retryAt ?? 0);
  if (!at || record.status === "connected") return "";
  const seconds = Math.round((at - Date.now()) / 1000);
  if (seconds <= 0) return "Trying again now";
  if (seconds < 60) return `Trying again in ${seconds}s`;
  return `Trying again in ${Math.round(seconds / 60)} min`;
}

/* -- one server ------------------------------------------------------------- */

function ServerRow({ record, busy, tools, onConnect, onDisconnect, onTools, onEdit, onRemove }) {
  const status = statusOf(record);
  const Glyph = record.type === "http" ? Globe : Terminal;
  const connected = record.status === "connected";

  // The connect path already appends the stderr tail to the message, so the
  // first line is the failure and everything after it is the server's own
  // output - which is usually the only thing that says what went wrong.
  const lines = String(record.error ?? "").split("\n");
  const headline = lines[0] ?? "";
  const output = lines.slice(1).join("\n").trim();

  return (
    <div className="flex animate-slide-up flex-col gap-2 rounded-2xl fill-control px-3 py-2.5">
      <div className="flex items-center gap-2.5">
        <span className="grid size-7 shrink-0 place-items-center rounded-full fill-secondary text-muted-foreground">
          <Glyph className="size-3.5" aria-hidden="true" />
        </span>

        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-[13px] text-foreground">{record.name || record.id}</span>
            <Badge size="sm" variant={status.variant}>
              {status.label}
            </Badge>
            {connected ? (
              <Badge size="sm">
                {record.toolCount} {record.toolCount === 1 ? "tool" : "tools"}
              </Badge>
            ) : null}
          </div>
          <p
            className="truncate font-mono text-[11px] text-muted-foreground"
            title={describe(record)}
          >
            {describe(record) || "Nothing configured to start"}
          </p>
        </div>

        {connected ? (
          <Button variant="subtle" size="xs" disabled={busy} onClick={onDisconnect}>
            <PowerOff />
            Disconnect
          </Button>
        ) : (
          <Button variant="subtle" size="xs" disabled={busy} onClick={onConnect}>
            {busy ? <Spinner size="sm" /> : <Play />}
            Connect
          </Button>
        )}
        <Button variant="subtle" size="xs" onClick={onTools}>
          <Wrench />
          Tools
        </Button>
        <IconButton size="lg" label={`Edit ${record.name || record.id}`} onClick={onEdit}>
          <Pencil />
        </IconButton>
        <IconButton size="lg" label={`Remove ${record.name || record.id}`} onClick={onRemove}>
          <Trash2 />
        </IconButton>
      </div>

      <Collapse open={Boolean(headline)}>
        <div className="flex flex-col gap-1.5 pl-9">
          <p className="text-[11px] leading-relaxed text-destructive-ink">
            {headline}
            {retryIn(record) ? (
              <span className="text-muted-foreground"> {retryIn(record)}.</span>
            ) : null}
          </p>
          {output ? (
            <Disclosure label="Server output">
              <Mono>{output}</Mono>
            </Disclosure>
          ) : null}
        </div>
      </Collapse>

      {/* Read with optional chaining throughout: the body stays mounted for
          the closing fold after `tools` has already been cleared. */}
      <Collapse open={Boolean(tools)}>
        <div className="pl-9">
          {tools?.loading ? (
            <div className="flex items-center gap-2 py-2 text-[11px] text-muted-foreground">
              <Spinner size="sm" />
              Asking the server what it provides
            </div>
          ) : tools?.rows?.length ? (
            <div className="flex animate-fade-in flex-col gap-1.5 py-1">
              {tools.rows.map((tool) => (
                <div key={tool.id ?? tool.name} className="min-w-0">
                  <p className="truncate font-mono text-[11px] text-foreground">{tool.name}</p>
                  <p className="text-[11px] leading-relaxed text-muted-foreground">
                    {tool.description || "The server documents no description for this tool."}
                  </p>
                </div>
              ))}
            </div>
          ) : tools?.error ? (
            <p className="py-2 text-[11px] leading-relaxed text-destructive-ink">{tools.error}</p>
          ) : (
            <p className="py-2 text-[11px] leading-relaxed text-muted-foreground">
              This server is connected and publishes no tools. There is nothing here for a model to
              call, which usually means the wrong command or the wrong URL.
            </p>
          )}
        </div>
      </Collapse>
    </div>
  );
}

/* -- the editor ------------------------------------------------------------- */

function TestReport({ report }) {
  if (!report) return null;

  return (
    <div className="flex animate-slide-up flex-col gap-2 rounded-xl fill-whisper px-3 py-2.5">
      <div className="flex items-start gap-2">
        {report.ok ? (
          <CircleCheck className="mt-px size-3.5 shrink-0 text-success-ink" aria-hidden="true" />
        ) : (
          <CircleAlert
            className="mt-px size-3.5 shrink-0 text-destructive-ink"
            aria-hidden="true"
          />
        )}
        <div className="min-w-0 flex-1 text-[11px] leading-relaxed text-muted-foreground">
          {report.ok ? (
            <>
              <span className="text-foreground">
                {report.serverInfo?.name || "The server"} answered in {report.latencyMs}ms
              </span>{" "}
              with {report.tools.length} {report.tools.length === 1 ? "tool" : "tools"}.
            </>
          ) : (
            <span className="text-destructive-ink">{report.error}</span>
          )}
        </div>
      </div>

      {report.tools.length ? (
        <Disclosure label={`Tools it found (${report.tools.length})`}>
          <div className="flex flex-col gap-1.5">
            {report.tools.map((tool) => (
              <div key={tool.name} className="min-w-0">
                <p className="truncate font-mono text-[11px] text-foreground">{tool.name}</p>
                <p className="text-[11px] leading-relaxed text-muted-foreground">
                  {tool.description}
                </p>
              </div>
            ))}
          </div>
        </Disclosure>
      ) : null}

      {report.stderr ? (
        <Disclosure label="Server output" defaultOpen={!report.ok}>
          <Mono>{report.stderr}</Mono>
        </Disclosure>
      ) : null}
    </div>
  );
}

function ServerDialog({ open, draft, editingId, onChange, onClose, onSaved }) {
  const { toast } = useToast();
  const [saving, setSaving] = React.useState(false);
  const [testing, setTesting] = React.useState(false);
  const [report, setReport] = React.useState(null);
  const [error, setError] = React.useState(null);

  React.useEffect(() => {
    if (open) {
      setReport(null);
      setError(null);
    }
  }, [open, editingId]);

  if (!draft) return null;
  const set = (patch) => onChange({ ...draft, ...patch });
  const stdio = draft.type !== "http";

  async function test() {
    const problem = validate(draft);
    if (problem) {
      setError(problem);
      return;
    }
    setTesting(true);
    setError(null);
    try {
      setReport(await mcp.test(toRecord(draft)));
    } catch (failure) {
      setError(failure.message);
    } finally {
      setTesting(false);
    }
  }

  async function save() {
    const problem = validate(draft);
    if (problem) {
      setError(problem);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const record = toRecord(draft);
      if (editingId) await mcp.update(editingId, record);
      else await mcp.add(record);
      toast({
        variant: "success",
        title: `${record.name} ${editingId ? "updated" : "added"}`,
        description: editingId
          ? "The server was stopped so the new configuration takes effect. Connect it again when you are ready."
          : "Connect it to start the server and read its tools.",
      });
      await onSaved();
      onClose();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()} size="lg">
      <DialogTitle>{editingId ? `Edit ${draft.name || "server"}` : "Add MCP server"}</DialogTitle>
      <DialogDescription>
        A local process Inertia starts and talks to over its standard input, or an HTTP endpoint it
        calls.
      </DialogDescription>

      <DialogBody className="mt-4">
        <ScrollArea className="max-h-[min(30rem,60dvh)] pr-1" fade>
          <div className="flex flex-col gap-4">
            <Field label="Name">
              <Input
                size="sm"
                value={draft.name}
                placeholder="Playwright"
                onChange={(e) => set({ name: e.target.value })}
              />
            </Field>

            <Field label="Transport">
              <Segmented
                value={draft.type}
                onChange={(type) => set({ type })}
                label="Transport"
                options={[
                  { value: "stdio", label: "Local process", icon: <Terminal /> },
                  { value: "http", label: "HTTP endpoint", icon: <Globe /> },
                ]}
              />
            </Field>

            {/* Keyed on the transport so its fields arrive as a set rather
                than a command box turning into a URL box. */}
            <div key={draft.type} className="flex animate-fade-in flex-col gap-4">
              {stdio ? (
                <>
                  <Field label="Command" hint="The executable only. Its arguments go below.">
                    <Input
                      size="sm"
                      className="font-mono"
                      value={draft.command}
                      placeholder="npx"
                      onChange={(e) => set({ command: e.target.value })}
                    />
                  </Field>

                  <Field
                    label="Arguments"
                    hint="One per row, in order. Kept separate so an argument containing a space stays one argument."
                  >
                    <ListEditor
                      value={draft.args}
                      onChange={(args) => set({ args })}
                      placeholder="-y"
                      addLabel="Add argument"
                    />
                  </Field>

                  <Field
                    label="Working directory"
                    hint="Optional. Where the process is started from."
                  >
                    <Input
                      size="sm"
                      className="font-mono"
                      value={draft.cwd}
                      placeholder="C:\\projects\\tools"
                      onChange={(e) => set({ cwd: e.target.value })}
                    />
                  </Field>

                  <Field label="Environment" hint={SECRET_SYNTAX}>
                    <SecretKeyValueEditor
                      value={draft.env}
                      onChange={(env) => set({ env })}
                      keyPlaceholder="GITHUB_TOKEN"
                      valuePlaceholder="{secret:GITHUB_TOKEN}"
                      addLabel="Add variable"
                    />
                  </Field>
                </>
              ) : (
                <>
                  <Field label="URL">
                    <Input
                      size="sm"
                      className="font-mono"
                      value={draft.url}
                      placeholder="https://mcp.example.com/mcp"
                      onChange={(e) => set({ url: e.target.value })}
                    />
                  </Field>

                  <Field label="Headers" hint={SECRET_SYNTAX}>
                    <SecretKeyValueEditor
                      value={draft.headers}
                      onChange={(headers) => set({ headers })}
                      keyPlaceholder="Authorization"
                      valuePlaceholder="Bearer {secret:MY_TOKEN}"
                      addLabel="Add header"
                    />
                  </Field>
                </>
              )}
            </div>

            <Field
              label="Call timeout"
              hint="Seconds to wait for one tool call before giving up. Leave empty for the default of 60. A server that reports progress while it works is given more time automatically, so this is for the ones that go quiet."
            >
              <Input
                size="sm"
                type="number"
                min="0"
                value={draft.timeoutMs ? String(Math.round(draft.timeoutMs / 1000)) : ""}
                placeholder="60"
                onChange={(e) =>
                  set({ timeoutMs: Math.max(0, Math.round(Number(e.target.value) || 0)) * 1000 })
                }
              />
            </Field>

            <ToggleRow
              label="Enabled"
              description="A disabled server stays on disk, is not started at launch and offers no tools."
              checked={draft.enabled !== false}
              onCheckedChange={(enabled) => set({ enabled })}
            />

            <TestReport report={report} />

            {error ? (
              <p className="animate-fade-in text-[11px] leading-relaxed text-destructive-ink">
                {error}
              </p>
            ) : null}
          </div>
        </ScrollArea>
      </DialogBody>

      <DialogFooter>
        <Button variant="subtle" size="sm" className="sm:mr-auto" disabled={testing} onClick={test}>
          {testing ? <Spinner size="sm" /> : <Play />}
          Test
        </Button>
        <Button variant="secondary" size="sm" onClick={onClose} disabled={saving}>
          Cancel
        </Button>
        <Button variant="primary" size="sm" onClick={save} disabled={saving}>
          {saving ? <Spinner size="sm" /> : null}
          {editingId ? "Save changes" : "Add server"}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

/* -- the screen ------------------------------------------------------------- */

function McpScreen({ query, addToken }) {
  const { toast } = useToast();

  const [servers, setServers] = React.useState([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState(null);
  const [busyId, setBusyId] = React.useState(null);
  const [tools, setTools] = React.useState({});

  const [draft, setDraft] = React.useState(null);
  const [editingId, setEditingId] = React.useState(null);
  const [removing, setRemoving] = React.useState(null);

  const reload = React.useCallback(async () => {
    try {
      setServers(await mcp.list());
      setError(null);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    reload();
  }, [reload]);

  // The primary button lives in the view header. The ref keeps a stale token
  // from popping the dialog open when the tab is remounted.
  const seenToken = React.useRef(addToken);
  React.useEffect(() => {
    if (addToken === seenToken.current) return;
    seenToken.current = addToken;
    setEditingId(null);
    setDraft(blank());
  }, [addToken]);

  async function connect(record) {
    setBusyId(record.id);
    try {
      const result = await mcp.connect(record.id);
      if (!result.ok) {
        toast({
          variant: "danger",
          title: `${record.name} did not start`,
          description: result.error,
        });
      }
    } catch (failure) {
      toast({ variant: "danger", title: "Could not start that", description: failure.message });
    } finally {
      setBusyId(null);
      await reload();
    }
  }

  async function disconnect(record) {
    setBusyId(record.id);
    try {
      await mcp.disconnect(record.id);
      setTools((current) => ({ ...current, [record.id]: undefined }));
    } catch (failure) {
      toast({ variant: "danger", title: "Could not stop that", description: failure.message });
    } finally {
      setBusyId(null);
      await reload();
    }
  }

  async function toggleTools(record) {
    if (tools[record.id]) {
      setTools((current) => ({ ...current, [record.id]: undefined }));
      return;
    }
    setTools((current) => ({ ...current, [record.id]: { loading: true, rows: [] } }));
    try {
      const rows = await mcp.tools(record.id);
      setTools((current) => ({ ...current, [record.id]: { loading: false, rows, error: "" } }));
    } catch (failure) {
      // Swallowing this made a server that could not be asked look exactly like
      // a server that answered with nothing, which are opposite problems.
      setTools((current) => ({
        ...current,
        [record.id]: { loading: false, rows: [], error: failure.message },
      }));
    }
  }

  async function confirmRemove() {
    const doomed = removing;
    if (!doomed) return;
    try {
      await mcp.remove(doomed.id);
      toast({ variant: "warning", title: `${doomed.name || doomed.id} removed` });
    } catch (failure) {
      toast({ variant: "danger", title: "Could not remove that", description: failure.message });
    } finally {
      setRemoving(null);
      await reload();
    }
  }

  const filtered = React.useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return servers;
    return servers.filter((record) =>
      `${record.name} ${describe(record)}`.toLowerCase().includes(needle)
    );
  }, [servers, query]);

  return (
    <>
      <ScrollArea className={cn("min-h-0 flex-1 pt-2 pb-8", GUTTER)}>
        {/* Mounted fresh on every tab switch (the view keys on the kind), so
            the arrival is the tab change. */}
        <Section
          className="animate-fade-in"
          title="Servers"
          count={filtered.length}
          description="Every connected server's tools are offered to every agent. A server that fails keeps its record, so the error is still on screen when you come back to fix it."
        >
          {loading ? (
            <div className="flex items-center gap-2 py-6 text-[13px] text-muted-foreground">
              <Spinner size="sm" />
              Reading the workspace folder
            </div>
          ) : error ? (
            <EmptyState
              icon={CircleAlert}
              title="That folder could not be read"
              description={error}
            />
          ) : filtered.length ? (
            <div className="flex flex-col gap-1.5">
              {filtered.map((record) => (
                <ServerRow
                  key={record.id}
                  record={record}
                  busy={busyId === record.id}
                  tools={tools[record.id]}
                  onConnect={() => connect(record)}
                  onDisconnect={() => disconnect(record)}
                  onTools={() => toggleTools(record)}
                  onEdit={() => {
                    setEditingId(record.id);
                    setDraft(toDraft(record));
                  }}
                  onRemove={() => setRemoving(record)}
                />
              ))}
            </div>
          ) : servers.length ? (
            <EmptyState
              icon={SearchX}
              title="Nothing matches"
              description={`None of your ${servers.length} servers match that search.`}
            />
          ) : (
            <EmptyState
              icon={Server}
              title="No MCP servers yet"
              description="Point Inertia at an MCP server and its tools become available to every agent. A local one is started as a child process; a remote one is called over HTTP."
              action={
                <Button
                  variant="primary"
                  size="sm"
                  onClick={() => {
                    setEditingId(null);
                    setDraft(blank());
                  }}
                >
                  <Server />
                  Add MCP server
                </Button>
              }
            />
          )}
        </Section>
      </ScrollArea>

      <ServerDialog
        open={Boolean(draft)}
        draft={draft}
        editingId={editingId}
        onChange={setDraft}
        onClose={() => {
          setDraft(null);
          setEditingId(null);
        }}
        onSaved={reload}
      />

      <ConfirmDialog
        open={Boolean(removing)}
        onOpenChange={(open) => !open && setRemoving(null)}
        destructive
        title={removing ? `Remove ${removing.name || removing.id}?` : "Remove?"}
        description="The server is stopped, its tools stop being offered to every agent, and its record is deleted from the workspace folder."
        confirmLabel="Remove"
        onConfirm={confirmRemove}
      />
    </>
  );
}

export const MCP_KIND = {
  id: "mcp",
  collection: "plugins.mcp",
  label: "MCP",
  icon: Server,
  noun: "MCP server",
  nounPlural: "MCP servers",
  addLabel: "Add MCP server",
  searchPlaceholder: "Search servers",
  Screen: McpScreen,
};
