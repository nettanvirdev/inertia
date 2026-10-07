import * as React from "react";
import {
  ChevronRight,
  CircleAlert,
  FileJson,
  Globe,
  Play,
  SearchX,
  Trash2,
  TriangleAlert,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { openapi } from "@/lib/integrations";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
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
import { Disclosure, Field, Mono, SECRET_SYNTAX, Section, SecretSelect } from "../fields";

/**
 * OpenAPI imports.
 *
 * Point Inertia at a spec and every operation in it can become a tool, which is
 * the cheapest real capability there is - most software a person already pays
 * for describes itself in one of these documents.
 *
 * Two things on this screen exist because of what goes wrong. The warnings from
 * an import are shown rather than swallowed, because a spec with references the
 * parser could not resolve produces tools whose arguments are undescribed and
 * which therefore fail at call time; and the operation list defaults to reads
 * only, because a model handed three hundred write operations will eventually
 * use one.
 */

const METHOD_VARIANT = {
  GET: "info",
  DELETE: "danger",
  POST: "warning",
  PUT: "warning",
  PATCH: "warning",
};

const AUTH_OPTIONS = [
  { value: "none", label: "No authentication" },
  { value: "bearer", label: "Bearer token" },
  { value: "header", label: "API key in a header" },
  { value: "query", label: "API key in the query string" },
  { value: "basic", label: "Basic auth" },
];

/** The shape `spec.buildRequest` reads, with the references left unresolved. */
function normaliseAuth(auth) {
  const type = auth?.type ?? "none";
  return {
    type,
    token: auth?.token ?? "",
    name: auth?.name ?? "",
    value: auth?.value ?? "",
    username: auth?.username ?? "",
    password: auth?.password ?? "",
  };
}

function reference(name) {
  return name ? `{secret:${name}}` : "";
}

/** The secret a `{secret:NAME}` value names, so the picker can show it back. */
function referencedName(value) {
  const match = /^\{secret:([^}]+)\}$/.exec(String(value ?? "").trim());
  return match ? match[1] : "";
}

/* -- what the spec says about itself ----------------------------------------- */

/**
 * The server variables the spec declares, as fields.
 *
 * A templated server URL - `https://{region}.api.example.com` - is the spec
 * telling the user there is a choice to make, and the app used to make it for
 * them by taking the string literally, braces and all. Nothing resolved and the
 * DNS error said nothing about why. Showing the variables is the smallest
 * honest version of this: the spec's own default is pre-filled, an enum becomes
 * a picker, and changing one rebuilds the base URL rather than asking the user
 * to edit two things that mean the same thing.
 */
function ServerVariables({ servers, values, onChange }) {
  const variables = [];
  const seen = new Set();
  for (const server of servers ?? []) {
    for (const variable of server.variables ?? []) {
      if (seen.has(variable.name)) continue;
      seen.add(variable.name);
      variables.push(variable);
    }
  }
  if (!variables.length) return null;

  return (
    <div className="flex flex-col gap-3">
      <div>
        <h3 className="text-[11px] font-semibold text-muted-foreground">Server variables</h3>
        <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
          This spec's server URL is a template. These fill it in, and the base URL above is rebuilt
          from them when you save.
        </p>
      </div>

      {variables.map((variable) => (
        <Field
          key={variable.name}
          label={variable.name}
          hint={
            variable.description || (variable.default ? `Defaults to ${variable.default}.` : "")
          }
        >
          {variable.enum?.length ? (
            <Select
              value={values[variable.name] ?? variable.default ?? variable.enum[0]}
              onChange={(value) => onChange({ ...values, [variable.name]: value })}
              options={variable.enum.map((option) => ({ value: option, label: option }))}
              ariaLabel={variable.name}
            />
          ) : (
            <Input
              size="sm"
              className="font-mono"
              value={values[variable.name] ?? ""}
              placeholder={variable.default || variable.name}
              onChange={(e) => onChange({ ...values, [variable.name]: e.target.value })}
            />
          )}
        </Field>
      ))}

      {servers.length > 1 ? (
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          The spec offers {servers.length} servers. The base URL above is built from the first;
          paste another in by hand if you want a different one.
        </p>
      ) : null}
    </div>
  );
}

/**
 * What the spec asks for at the door, and whether this app can provide it.
 *
 * An import used to look ready no matter what the document declared, and a spec
 * whose only scheme is OAuth2 then failed every call with a 401 from inside an
 * agent turn where nobody could see the cause. Saying so on the settings screen
 * costs one paragraph and saves that entire discovery.
 */
function SecurityNotice({ security }) {
  const supported = security?.supported ?? [];
  const unsupported = security?.unsupported ?? [];
  if (!supported.length && !unsupported.length) return null;

  const blocked = unsupported.length > 0 && supported.length === 0;

  return (
    <div className="flex items-start gap-2 rounded-xl fill-whisper px-3 py-2.5">
      {blocked ? (
        <CircleAlert className="mt-px size-3.5 shrink-0 text-warning-ink" aria-hidden="true" />
      ) : (
        <Globe className="mt-px size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      )}
      <div className="min-w-0 flex-1">
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          {blocked
            ? "This API's authentication is not something Inertia can set up. Calls will be refused until you supply a credential another way."
            : "This API accepts authentication Inertia can configure. Pick the matching option below."}
        </p>
        <ul className="mt-1.5 flex flex-col gap-1">
          {[...supported, ...unsupported].map((scheme) => (
            <li key={scheme.name} className="flex items-start gap-1.5">
              <Badge size="sm" variant={scheme.supported ? "info" : "warning"}>
                {scheme.name}
              </Badge>
              <span className="text-[11px] leading-relaxed text-muted-foreground">
                {scheme.summary}
              </span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

/* -- import ----------------------------------------------------------------- */

function ImportDialog({ open, onClose, onImported }) {
  const { toast } = useToast();
  const [source, setSource] = React.useState("url");
  const [name, setName] = React.useState("");
  const [url, setUrl] = React.useState("");
  const [text, setText] = React.useState("");
  const [baseUrl, setBaseUrl] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState(null);

  React.useEffect(() => {
    if (!open) return;
    setSource("url");
    setName("");
    setUrl("");
    setText("");
    setBaseUrl("");
    setError(null);
  }, [open]);

  async function run() {
    setBusy(true);
    setError(null);
    try {
      const result = await openapi.import({
        name: name.trim(),
        url: source === "url" ? url.trim() : "",
        text: source === "paste" ? text : "",
        baseUrl: baseUrl.trim(),
      });
      toast({
        variant: "success",
        title: `${result.record.name} imported`,
        description: `${result.operations.length} operations found${
          result.warnings?.length ? `, with ${result.warnings.length} warnings to read` : ""
        }.`,
      });
      await onImported(result.record.id);
      onClose();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()} size="lg">
      <DialogTitle>Import an API</DialogTitle>
      <DialogDescription>
        The document is parsed, its references resolved and the result stored in the workspace
        folder. Nothing is fetched again afterwards, so a spec that changes needs importing again.
      </DialogDescription>

      <DialogBody className="mt-4">
        <ScrollArea className="max-h-[min(28rem,58dvh)] pr-1" fade>
          <div className="flex flex-col gap-4">
            <Field label="Where the spec comes from">
              <Segmented
                value={source}
                onChange={setSource}
                label="Spec source"
                options={[
                  { value: "url", label: "From a URL", icon: <Globe /> },
                  { value: "paste", label: "Paste a document", icon: <FileJson /> },
                ]}
              />
            </Field>

            {/* Keyed on the source so switching swaps one field for another
                with a fade rather than rewriting the label under the cursor. */}
            <div key={source} className="animate-fade-in">
              {source === "url" ? (
                <Field
                  label="Spec URL"
                  hint="JSON or YAML. A URL that redirects is refused rather than followed."
                >
                  <Input
                    size="sm"
                    className="font-mono"
                    value={url}
                    placeholder="https://api.example.com/openapi.json"
                    onChange={(e) => setUrl(e.target.value)}
                  />
                </Field>
              ) : (
                <Field label="Spec document" hint="JSON or YAML, pasted whole.">
                  <Textarea
                    rows={8}
                    maxRows={16}
                    autoResize
                    className="font-mono text-[11px]"
                    value={text}
                    placeholder='{ "openapi": "3.1.0", "info": { }, "paths": { } }'
                    onChange={(e) => setText(e.target.value)}
                  />
                </Field>
              )}
            </div>

            <Field label="Name" hint="Optional. The spec's own title is used when this is empty.">
              <Input
                size="sm"
                value={name}
                placeholder="Statuspage"
                onChange={(e) => setName(e.target.value)}
              />
            </Field>

            <Field
              label="Base URL"
              hint="Optional. Specs ship with localhost or a relative path in them more often than not, so whatever you put here wins."
            >
              <Input
                size="sm"
                className="font-mono"
                value={baseUrl}
                placeholder="https://api.example.com/v1"
                onChange={(e) => setBaseUrl(e.target.value)}
              />
            </Field>

            {error ? (
              <p className="animate-fade-in text-[11px] leading-relaxed text-destructive-ink">
                {error}
              </p>
            ) : null}
          </div>
        </ScrollArea>
      </DialogBody>

      <DialogFooter>
        <Button variant="secondary" size="sm" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button variant="primary" size="sm" onClick={run} disabled={busy}>
          {busy ? <Spinner size="sm" /> : null}
          Import
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

/* -- one import ------------------------------------------------------------- */

function AuthFields({ auth, onChange }) {
  const set = (patch) => onChange({ ...auth, ...patch });

  if (auth.type === "none") return null;

  if (auth.type === "basic") {
    return (
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Username">
          <Input
            size="sm"
            className="font-mono"
            value={auth.username}
            placeholder="api"
            onChange={(e) => set({ username: e.target.value })}
          />
        </Field>
        <Field label="Password" hint="Pick a secret rather than typing the value here.">
          <SecretSelect
            value={referencedName(auth.password)}
            onChange={(secret) => set({ password: reference(secret) })}
          />
        </Field>
      </div>
    );
  }

  if (auth.type === "bearer") {
    return (
      <Field label="Token" hint={SECRET_SYNTAX}>
        <SecretSelect
          value={referencedName(auth.token)}
          onChange={(secret) => set({ token: reference(secret) })}
        />
      </Field>
    );
  }

  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <Field label={auth.type === "query" ? "Parameter name" : "Header name"}>
        <Input
          size="sm"
          className="font-mono"
          value={auth.name}
          placeholder={auth.type === "query" ? "api_key" : "X-API-Key"}
          onChange={(e) => set({ name: e.target.value })}
        />
      </Field>
      <Field label="Value" hint={SECRET_SYNTAX}>
        <SecretSelect
          value={referencedName(auth.value)}
          onChange={(secret) => set({ value: reference(secret) })}
        />
      </Field>
    </div>
  );
}

/** Call one operation for real, with arguments the user typed. */
function TestPanel({ record, operations }) {
  const [operationId, setOperationId] = React.useState("");
  const [args, setArgs] = React.useState("{}");
  const [busy, setBusy] = React.useState(false);
  const [result, setResult] = React.useState(null);
  const [error, setError] = React.useState(null);

  const options = React.useMemo(
    () =>
      operations.map((operation) => ({
        value: operation.id,
        label: `${operation.method} ${operation.path}`,
        description: operation.summary || undefined,
      })),
    [operations]
  );

  const chosen = operations.find((operation) => operation.id === operationId) ?? null;

  async function run() {
    if (!chosen) return;
    let parsed;
    try {
      parsed = args.trim() ? JSON.parse(args) : {};
    } catch (failure) {
      setError(`Those arguments are not valid JSON: ${failure.message}`);
      return;
    }
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      setResult(await openapi.test(record.id, chosen.id, parsed));
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <p className="max-w-[80ch] text-[11px] leading-relaxed text-muted-foreground">
        A base URL missing a trailing path segment, or a key in the wrong header, fails later inside
        an agent turn as a confusing tool error. Calling one operation here fails now, in front of
        the person who can fix it, with the API's own words on screen.
      </p>

      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Operation">
          <Select
            value={operationId}
            onChange={setOperationId}
            options={options}
            placeholder="Choose an operation"
            ariaLabel="Operation to call"
          />
        </Field>
        <Field
          label="Arguments"
          hint="JSON. Path and query parameters go at the top level; a request body goes under `body`."
        >
          <Textarea
            rows={3}
            autoResize
            maxRows={10}
            className="font-mono text-[11px]"
            value={args}
            placeholder='{ "id": "123" }'
            onChange={(e) => setArgs(e.target.value)}
          />
        </Field>
      </div>

      <Button
        variant="subtle"
        size="sm"
        className="self-start"
        disabled={busy || !chosen}
        onClick={run}
      >
        {busy ? <Spinner size="sm" /> : <Play />}
        Call it
      </Button>

      {error ? (
        <p className="animate-fade-in text-[11px] leading-relaxed text-destructive-ink">{error}</p>
      ) : null}
      {result ? <Mono className="max-h-72 animate-slide-up">{result.output}</Mono> : null}
    </div>
  );
}

function ImportCard({ record, expanded, onToggle, onChanged, onRemove }) {
  const { toast } = useToast();

  const [operations, setOperations] = React.useState([]);
  const [loading, setLoading] = React.useState(false);
  const [baseUrl, setBaseUrl] = React.useState(record.baseUrl ?? "");
  const [auth, setAuth] = React.useState(() => normaliseAuth(record.auth));
  const [details, setDetails] = React.useState(null);
  const [serverVariables, setServerVariables] = React.useState(() => record.serverVariables ?? {});
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState(null);

  React.useEffect(() => {
    if (!expanded) return undefined;
    let alive = true;
    setLoading(true);
    openapi
      .operations(record.id)
      .then((rows) => alive && setOperations(rows))
      .catch((failure) => alive && setError(failure.message))
      .finally(() => alive && setLoading(false));

    // A failure here costs the servers and security panels, not the card. The
    // stored spec can go missing - somebody tidied the folder - and losing the
    // whole row over it would take the delete button with it.
    openapi
      .details(record.id)
      .then((answer) => alive && answer && setDetails(answer))
      .catch(() => {});

    return () => {
      alive = false;
    };
  }, [expanded, record.id]);

  // Saving a server variable makes main rebuild the base URL, so the field has
  // to follow the record rather than the value it was first mounted with.
  React.useEffect(() => {
    setBaseUrl(record.baseUrl ?? "");
  }, [record.baseUrl]);

  const enabledCount = operations.filter((operation) => operation.enabled).length;

  function setOperationEnabled(id, enabled) {
    setOperations((rows) => rows.map((row) => (row.id === id ? { ...row, enabled } : row)));
  }

  async function saveOperations() {
    setSaving(true);
    setError(null);
    try {
      await openapi.setOperations(
        record.id,
        operations.map((operation) => ({ id: operation.id, enabled: operation.enabled }))
      );
      toast({
        variant: "success",
        title: `${record.name} updated`,
        description: `${enabledCount} of ${operations.length} operations are offered to your agents.`,
      });
      await onChanged();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setSaving(false);
    }
  }

  async function saveSettings() {
    setSaving(true);
    setError(null);
    try {
      // The base URL is sent explicitly whenever the user edited it, and left
      // out when they only changed a variable, because main rebuilds it from
      // the variables in that case and the field they are looking at is about
      // to be replaced by the answer.
      const edited = baseUrl.trim() !== String(record.baseUrl ?? "").trim();
      // Variables are only sent when the spec declares some. Sending an empty
      // set would tell main to rebuild the base URL from the spec, which would
      // quietly undo a base URL the user had typed in themselves.
      const templated = (details?.servers ?? []).some((server) => server.variables?.length);
      await openapi.update(record.id, {
        ...(edited ? { baseUrl } : {}),
        ...(templated ? { serverVariables } : {}),
        auth,
      });
      toast({ variant: "success", title: `${record.name} updated` });
      await onChanged();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setSaving(false);
    }
  }

  async function toggleEnabled(enabled) {
    try {
      await openapi.update(record.id, { enabled });
      await onChanged();
    } catch (failure) {
      toast({ variant: "danger", title: "Could not save that", description: failure.message });
    }
  }

  return (
    <div className="flex animate-slide-up flex-col gap-3 rounded-2xl fill-control px-3 py-2.5">
      <div className="flex items-center gap-2.5">
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={expanded}
          className={cn(
            "flex min-w-0 flex-1 items-center gap-2.5 rounded-xl text-left outline-none",
            "focus-visible:fill-control-hover"
          )}
        >
          <span className="grid size-7 shrink-0 place-items-center rounded-full fill-secondary text-muted-foreground">
            <ChevronRight
              className={cn(
                "size-3.5 transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
                expanded && "rotate-90"
              )}
              aria-hidden="true"
            />
          </span>
          <span className="min-w-0 flex-1">
            <span className="flex min-w-0 items-center gap-2">
              <span className="truncate text-[13px] text-foreground">{record.name}</span>
              {record.auth?.type && record.auth.type !== "none" ? (
                <Badge size="sm">{record.auth.type}</Badge>
              ) : null}
              {record.warnings?.length ? (
                <Badge size="sm" variant="warning">
                  {record.warnings.length} warnings
                </Badge>
              ) : null}
            </span>
            <span className="block truncate font-mono text-[11px] text-muted-foreground">
              {record.baseUrl || "No base URL set, so nothing can be called"}
            </span>
          </span>
        </button>

        <Switch
          size="sm"
          checked={record.enabled !== false}
          onCheckedChange={toggleEnabled}
          label={`${record.enabled === false ? "Enable" : "Disable"} ${record.name}`}
        />
        <IconButton size="lg" label={`Remove ${record.name}`} onClick={onRemove}>
          <Trash2 />
        </IconButton>
      </div>

      <Collapse open={expanded}>
        <div className="flex flex-col gap-5 pl-9">
          {record.warnings?.length ? (
            <div className="flex items-start gap-2 rounded-xl fill-whisper px-3 py-2.5">
              <TriangleAlert
                className="mt-px size-3.5 shrink-0 text-warning-ink"
                aria-hidden="true"
              />
              <div className="min-w-0 flex-1">
                <p className="text-[11px] leading-relaxed text-muted-foreground">
                  Parts of this spec did not come through cleanly. A reference that could not be
                  resolved leaves an operation with arguments nothing describes; an operation whose
                  body Inertia cannot build was skipped entirely; a templated server URL needs a
                  value before anything will resolve. Read them and you will know which.
                </p>
                <Disclosure
                  label={`Read the ${record.warnings.length} warnings`}
                  className="mt-1.5"
                >
                  <Mono>{record.warnings.join("\n")}</Mono>
                </Disclosure>
              </div>
            </div>
          ) : null}

          <SecurityNotice security={details?.security} />

          <div className="flex flex-col gap-3">
            <Field label="Base URL" hint="Where the calls actually go.">
              <Input
                size="sm"
                className="font-mono"
                value={baseUrl}
                placeholder="https://api.example.com/v1"
                onChange={(e) => setBaseUrl(e.target.value)}
              />
            </Field>

            <ServerVariables
              servers={details?.servers}
              values={serverVariables}
              onChange={setServerVariables}
            />

            <Field label="Authentication">
              <Select
                value={auth.type}
                onChange={(type) => setAuth({ ...auth, type })}
                options={AUTH_OPTIONS}
                ariaLabel="Authentication"
              />
            </Field>

            {/* A new scheme is a new set of fields: keyed so they arrive
                rather than have their labels rewritten in place. */}
            {auth.type === "none" ? null : (
              <div key={auth.type} className="animate-fade-in">
                <AuthFields auth={auth} onChange={setAuth} />
              </div>
            )}

            <Button
              variant="subtle"
              size="sm"
              className="self-start"
              disabled={saving}
              onClick={saveSettings}
            >
              Save connection
            </Button>
          </div>

          <div className="flex flex-col gap-2">
            <div className="flex items-center gap-2">
              <h3 className="text-[11px] font-semibold text-muted-foreground">
                Operations{" "}
                <span className="font-normal tabular-nums">
                  {enabledCount} of {operations.length}
                </span>
              </h3>
              <Button
                variant="subtle"
                size="xs"
                className="ml-auto"
                onClick={() =>
                  setOperations((rows) =>
                    rows.map((row) => ({
                      ...row,
                      enabled: row.callable !== false && row.method === "GET",
                    }))
                  )
                }
              >
                Reads only
              </Button>
              <Button
                variant="subtle"
                size="xs"
                onClick={() =>
                  setOperations((rows) =>
                    rows.map((row) => ({ ...row, enabled: row.callable !== false }))
                  )
                }
              >
                Enable all
              </Button>
              <Button variant="primary" size="xs" disabled={saving} onClick={saveOperations}>
                Save operations
              </Button>
            </div>

            <p className="max-w-[80ch] text-[11px] leading-relaxed text-muted-foreground">
              GETs start enabled and everything else starts off, on purpose: reading is what an
              agent does most and is the least dangerous thing to have available, and a model given
              three hundred write operations will eventually use one.
            </p>

            {loading ? (
              <div className="flex items-center gap-2 py-4 text-[11px] text-muted-foreground">
                <Spinner size="sm" />
                Reading the stored spec
              </div>
            ) : (
              <ScrollArea className="max-h-80 pr-1" fade>
                <div className="flex flex-col gap-0.5 animate-fade-in">
                  {operations.map((operation) => (
                    <div
                      key={operation.id}
                      className="flex items-center gap-2.5 rounded-xl px-2 py-1.5 transition-colors duration-150 ease-out hover:fill-control-hover"
                    >
                      <Badge size="sm" variant={METHOD_VARIANT[operation.method] ?? "neutral"}>
                        {operation.method}
                      </Badge>
                      <div className="min-w-0 flex-1">
                        <p className="truncate font-mono text-[11px] text-foreground">
                          {operation.path}
                        </p>
                        <p className="truncate text-[11px] text-muted-foreground">
                          {operation.callable === false
                            ? operation.reason
                            : operation.summary || operation.id}
                        </p>
                      </div>
                      {/* An operation Inertia cannot build a request for never
                          becomes a tool, so the switch would be a control that
                          silently does nothing. It says so instead. */}
                      {operation.callable === false ? (
                        <Badge size="sm" variant="warning">
                          Skipped
                        </Badge>
                      ) : (
                        <Switch
                          size="sm"
                          checked={operation.enabled}
                          onCheckedChange={(enabled) => setOperationEnabled(operation.id, enabled)}
                          label={`${operation.enabled ? "Disable" : "Enable"} ${operation.method} ${operation.path}`}
                        />
                      )}
                    </div>
                  ))}
                </div>
              </ScrollArea>
            )}
          </div>

          <div className="flex flex-col gap-2">
            <h3 className="text-[11px] font-semibold text-muted-foreground">Test a call</h3>
            <TestPanel record={record} operations={operations} />
          </div>

          {error ? (
            <p className="animate-fade-in text-[11px] leading-relaxed text-destructive-ink">
              {error}
            </p>
          ) : null}
        </div>
      </Collapse>
    </div>
  );
}

/* -- the screen ------------------------------------------------------------- */

function OpenApiScreen({ query, addToken }) {
  const { toast } = useToast();

  const [records, setRecords] = React.useState([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState(null);
  const [expanded, setExpanded] = React.useState(null);
  const [importing, setImporting] = React.useState(false);
  const [removing, setRemoving] = React.useState(null);

  const reload = React.useCallback(async () => {
    try {
      setRecords(await openapi.list());
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

  const seenToken = React.useRef(addToken);
  React.useEffect(() => {
    if (addToken === seenToken.current) return;
    seenToken.current = addToken;
    setImporting(true);
  }, [addToken]);

  async function confirmRemove() {
    const doomed = removing;
    if (!doomed) return;
    try {
      await openapi.remove(doomed.id);
      toast({ variant: "warning", title: `${doomed.name} removed` });
    } catch (failure) {
      toast({ variant: "danger", title: "Could not remove that", description: failure.message });
    } finally {
      setRemoving(null);
      await reload();
    }
  }

  const filtered = React.useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return records;
    return records.filter((record) =>
      `${record.name} ${record.description} ${record.baseUrl} ${record.sourceUrl}`
        .toLowerCase()
        .includes(needle)
    );
  }, [records, query]);

  return (
    <>
      <ScrollArea className="min-h-0 flex-1 px-4 pt-2 pb-8">
        {/* Mounted fresh on every tab switch (the view keys on the kind), so
            the arrival is the tab change. */}
        <Section className="animate-fade-in" title="Imported APIs" count={filtered.length}>
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
                <ImportCard
                  key={record.id}
                  record={record}
                  expanded={expanded === record.id}
                  onToggle={() =>
                    setExpanded((current) => (current === record.id ? null : record.id))
                  }
                  onChanged={reload}
                  onRemove={() => setRemoving(record)}
                />
              ))}
            </div>
          ) : records.length ? (
            <EmptyState
              icon={SearchX}
              title="Nothing matches"
              description={`None of your ${records.length} imported APIs match that search.`}
            />
          ) : (
            <EmptyState
              icon={FileJson}
              title="No APIs imported yet"
              description="Import a spec by URL or paste the document. Inertia resolves its references, keeps the result in the workspace folder, and turns the operations you allow into tools."
              action={
                <Button variant="primary" size="sm" onClick={() => setImporting(true)}>
                  <FileJson />
                  Import an API
                </Button>
              }
            />
          )}
        </Section>
      </ScrollArea>

      <ImportDialog
        open={importing}
        onClose={() => setImporting(false)}
        onImported={async (id) => {
          await reload();
          setExpanded(id);
        }}
      />

      <ConfirmDialog
        open={Boolean(removing)}
        onOpenChange={(open) => !open && setRemoving(null)}
        destructive
        title={removing ? `Remove ${removing.name}?` : "Remove?"}
        description="Every operation stops being offered as a tool, and both the record and the stored spec document are deleted from the workspace folder."
        confirmLabel="Remove"
        onConfirm={confirmRemove}
      />
    </>
  );
}

export const OPENAPI_KIND = {
  id: "openapi",
  collection: "plugins.openapi",
  label: "OpenAPI",
  icon: FileJson,
  noun: "API",
  nounPlural: "APIs",
  addLabel: "Import an API",
  searchPlaceholder: "Search APIs",
  Screen: OpenApiScreen,
};
