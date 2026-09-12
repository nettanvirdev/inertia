import * as React from "react";
import { ChevronRight, KeyRound, Plus, X } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useWorkspace } from "@/lib/workspace";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { IconButton } from "@/components/ui/icon-button";
import { Collapse } from "@/components/ui/collapse";

/**
 * The form furniture every plugin editor shares.
 *
 * Four editors that all look different would make four screens the user has to
 * learn separately, so the label/hint/error rhythm lives here once and each
 * kind only supplies its own fields.
 */

export function Field({ label, hint, error, htmlFor, children, className }) {
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      {label ? (
        <label htmlFor={htmlFor} className="text-[11px] font-medium text-muted-foreground">
          {label}
        </label>
      ) : null}
      {children}
      {error ? (
        <p className="text-[11px] text-destructive-ink">{error}</p>
      ) : hint ? (
        <p className="text-[11px] leading-relaxed text-muted-foreground">{hint}</p>
      ) : null}
    </div>
  );
}

/** A labelled switch on its own row - used for `enabled` in every editor. */
export function ToggleRow({ label, description, checked, onCheckedChange }) {
  return (
    <div className="flex items-center gap-3 rounded-xl fill-whisper px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <p className="text-[13px] text-foreground">{label}</p>
        {description ? (
          <p className="text-[11px] leading-relaxed text-muted-foreground">{description}</p>
        ) : null}
      </div>
      <Switch checked={checked} onCheckedChange={onCheckedChange} label={label} />
    </div>
  );
}

/* -- key/value pairs --------------------------------------------------- */

let rowSeq = 0;
const nextRowId = () => `kv-${(rowSeq += 1)}`;

function toRows(object) {
  return Object.entries(object ?? {}).map(([key, value]) => ({
    id: nextRowId(),
    key,
    value: String(value ?? ""),
  }));
}

function toObject(rows) {
  const out = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (key) out[key] = row.value;
  }
  return out;
}

/**
 * Environment variables and request headers are the same shape, so they get the
 * same editor. Rows are kept as a list rather than derived from the object on
 * every keystroke: an object cannot hold a half-typed key, and a row whose key
 * is momentarily blank would otherwise vanish under the cursor.
 */
export function KeyValueEditor({ value, onChange, keyPlaceholder = "KEY", valuePlaceholder = "value", addLabel = "Add row" }) {
  const [rows, setRows] = React.useState(() => toRows(value));

  const commit = (next) => {
    setRows(next);
    onChange?.(toObject(next));
  };

  return (
    <div className="flex flex-col gap-1.5">
      {rows.map((row, index) => (
        <div key={row.id} className="flex items-center gap-1.5">
          <Input
            size="sm"
            className="flex-1 font-mono"
            value={row.key}
            placeholder={keyPlaceholder}
            aria-label={`${keyPlaceholder} ${index + 1}`}
            onChange={(e) =>
              commit(rows.map((r) => (r.id === row.id ? { ...r, key: e.target.value } : r)))
            }
          />
          <Input
            size="sm"
            className="flex-1 font-mono"
            value={row.value}
            placeholder={valuePlaceholder}
            aria-label={`${valuePlaceholder} ${index + 1}`}
            onChange={(e) =>
              commit(rows.map((r) => (r.id === row.id ? { ...r, value: e.target.value } : r)))
            }
          />
          <IconButton
            size="lg"
            label={`Remove row ${index + 1}`}
            onClick={() => commit(rows.filter((r) => r.id !== row.id))}
          >
            <X />
          </IconButton>
        </div>
      ))}
      <Button
        variant="subtle"
        size="sm"
        className="self-start"
        onClick={() => commit([...rows, { id: nextRowId(), key: "", value: "" }])}
      >
        <Plus />
        {addLabel}
      </Button>
    </div>
  );
}

/* -- tags -------------------------------------------------------------- */

/** Tags are typed as one comma separated line; the record stores an array. */
export function TagsInput({ value, onChange, placeholder = "dev, qa" }) {
  const [text, setText] = React.useState(() => (value ?? []).join(", "));

  return (
    <Input
      size="sm"
      value={text}
      placeholder={placeholder}
      onChange={(e) => {
        setText(e.target.value);
        onChange?.(
          e.target.value
            .split(",")
            .map((t) => t.trim())
            .filter(Boolean)
        );
      }}
    />
  );
}

/* -- secrets ----------------------------------------------------------- */

/** The names of the secrets in the workspace vault. Never the values: a plugin
 *  record stores the NAME, so a workspace folder can be shared without leaking
 *  anything. */
export function useSecretNames() {
  const { client, configured } = useWorkspace();
  const [names, setNames] = React.useState([]);

  React.useEffect(() => {
    if (!configured) return undefined;
    let alive = true;
    client.secrets
      .list()
      .then((rows) => alive && setNames(rows.map((r) => r.name)))
      .catch(() => alive && setNames([]));
    return () => {
      alive = false;
    };
  }, [client, configured]);

  return names;
}

const NO_SECRET = "";

export function SecretSelect({ value, onChange, placeholder = "No secret" }) {
  const names = useSecretNames();

  // A name already on the record but missing from the vault still has to be
  // selectable, or opening the editor would silently drop it on save.
  const options = React.useMemo(() => {
    const all = new Set(names);
    if (value) all.add(value);
    return [
      { value: NO_SECRET, label: placeholder },
      ...[...all].sort().map((name) => ({
        value: name,
        label: name,
        icon: KeyRound,
        description: names.includes(name) ? undefined : "Not in this workspace yet",
      })),
    ];
  }, [names, value, placeholder]);

  return (
    <Select
      value={value ?? NO_SECRET}
      onChange={(next) => onChange?.(next)}
      options={options}
      placeholder={placeholder}
      ariaLabel="Stored secret"
    />
  );
}

/** The one sentence that explains the reference syntax, written once. */
export const SECRET_SYNTAX =
  "Write {secret:NAME} anywhere in a value and the stored secret is swapped in at the moment the server is started. The record on disk keeps the name, never the token.";

/**
 * Key/value rows whose value may carry a `{secret:NAME}` reference.
 *
 * The picker inserts rather than replaces, because the common case is not a
 * bare token: it is `Bearer {secret:X}`, or a connection string with the
 * password in the middle of it. Replacing would delete the part the user had
 * already typed and give them no way to say what they meant.
 */
export function SecretKeyValueEditor({
  value,
  onChange,
  keyPlaceholder = "KEY",
  valuePlaceholder = "value",
  addLabel = "Add row",
}) {
  const [rows, setRows] = React.useState(() => toRows(value));

  const commit = (next) => {
    setRows(next);
    onChange?.(toObject(next));
  };

  const insertSecret = (row, name) => {
    if (!name) return;
    const reference = `{secret:${name}}`;
    const current = row.value.trim();
    const next = current ? `${row.value} ${reference}` : reference;
    commit(rows.map((r) => (r.id === row.id ? { ...r, value: next } : r)));
  };

  return (
    <div className="flex flex-col gap-1.5">
      {rows.map((row, index) => (
        <div key={row.id} className="flex items-start gap-1.5">
          <Input
            size="sm"
            className="w-[9rem] shrink-0 font-mono"
            value={row.key}
            placeholder={keyPlaceholder}
            aria-label={`${keyPlaceholder} ${index + 1}`}
            onChange={(e) =>
              commit(rows.map((r) => (r.id === row.id ? { ...r, key: e.target.value } : r)))
            }
          />
          <div className="flex min-w-0 flex-1 flex-col gap-1.5">
            <Input
              size="sm"
              className="font-mono"
              value={row.value}
              placeholder={valuePlaceholder}
              aria-label={`${valuePlaceholder} ${index + 1}`}
              onChange={(e) =>
                commit(rows.map((r) => (r.id === row.id ? { ...r, value: e.target.value } : r)))
              }
            />
            <SecretSelect
              value=""
              placeholder="Insert a secret reference"
              onChange={(name) => insertSecret(row, name)}
            />
          </div>
          <IconButton
            size="lg"
            label={`Remove row ${index + 1}`}
            onClick={() => commit(rows.filter((r) => r.id !== row.id))}
          >
            <X />
          </IconButton>
        </div>
      ))}
      <Button
        variant="subtle"
        size="sm"
        className="self-start"
        onClick={() => commit([...rows, { id: nextRowId(), key: "", value: "" }])}
      >
        <Plus />
        {addLabel}
      </Button>
    </div>
  );
}

/* -- ordered lists ------------------------------------------------------ */

/**
 * A list of strings, one per row.
 *
 * Command arguments are edited this way rather than as one line because order
 * and whitespace both matter to a process being spawned, and a single text
 * field makes `--flag "a b"` indistinguishable from `--flag a b`.
 */
export function ListEditor({ value, onChange, placeholder = "value", addLabel = "Add item" }) {
  const items = Array.isArray(value) ? value : [];
  const commit = (next) => onChange?.(next);

  return (
    <div className="flex flex-col gap-1.5">
      {items.map((item, index) => (
        // Rows have no identity of their own, so the index is the key. It is
        // stable here because the only edits are append, replace and remove.
        <div key={`arg-${index}`} className="flex items-center gap-1.5">
          <span className="w-5 shrink-0 text-right font-mono text-[11px] text-muted-foreground">
            {index + 1}
          </span>
          <Input
            size="sm"
            className="flex-1 font-mono"
            value={item}
            placeholder={placeholder}
            aria-label={`Item ${index + 1}`}
            onChange={(e) =>
              commit(items.map((current, at) => (at === index ? e.target.value : current)))
            }
          />
          <IconButton
            size="lg"
            label={`Remove item ${index + 1}`}
            onClick={() => commit(items.filter((_, at) => at !== index))}
          >
            <X />
          </IconButton>
        </div>
      ))}
      <Button
        variant="subtle"
        size="sm"
        className="self-start"
        onClick={() => commit([...items, ""])}
      >
        <Plus />
        {addLabel}
      </Button>
    </div>
  );
}

/* -- page furniture ----------------------------------------------------- */

/** A titled block on a manager screen, with an optional count and control. */
export function Section({ title, count, action, description, children, className }) {
  return (
    <section className={cn("flex flex-col gap-2", className)}>
      <div className="flex items-center gap-3">
        <h2 className="flex items-center gap-2 text-[11px] font-semibold text-muted-foreground">
          {title}
          {count === undefined ? null : <span className="font-normal tabular-nums">{count}</span>}
        </h2>
        {action ? <div className="ml-auto flex items-center gap-2">{action}</div> : null}
      </div>
      {description ? (
        <p className="max-w-[80ch] text-[11px] leading-relaxed text-muted-foreground">
          {description}
        </p>
      ) : null}
      {children}
    </section>
  );
}

/**
 * A folded block of machine output - stderr, a response body, a warning list.
 *
 * Folded by default because it is diagnosis, not information: it matters
 * enormously the one time something is broken and is noise every other time.
 */
export function Disclosure({ label, children, defaultOpen = false, className }) {
  const [open, setOpen] = React.useState(defaultOpen);

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className={cn(
          "flex w-fit items-center gap-1 rounded-lg px-1.5 py-1 text-[11px] outline-none",
          "text-muted-foreground transition-colors duration-150 ease-out hover:text-foreground",
          "focus-visible:fill-control-hover"
        )}
      >
        {/* One chevron that turns, not two that swap: the turn is what says
            the same thing is opening rather than being replaced. */}
        <ChevronRight
          className={cn(
            "size-3 transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
            open && "rotate-90"
          )}
          aria-hidden="true"
        />
        {label}
      </button>
      <Collapse open={open}>{children}</Collapse>
    </div>
  );
}

/** Machine output, kept monospaced and scrollable in both directions. */
export function Mono({ children, className }) {
  return (
    <pre
      className={cn(
        "max-h-56 overflow-auto rounded-xl fill-whisper px-3 py-2.5",
        "font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-muted-foreground",
        className
      )}
    >
      {children}
    </pre>
  );
}
