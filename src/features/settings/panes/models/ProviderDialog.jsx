import * as React from "react";
import {
  CircleAlert,
  CircleCheck,
  Icon,
  KeyRound,
  Plug,
  Plus,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { useWorkspace } from "@/lib/workspace";
import {
  PRESETS,
  PROTOCOLS,
  PROTOCOL_LABELS,
  blankProvider,
  normalizeBaseUrl,
  protocolOf,
  validateProvider,
} from "@shared/providers";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import {
  Dialog,
  DialogBody,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { SettingsField } from "../../SettingsRow";
import { ModelsField } from "./ModelsField";
import { describeFailure, testProvider } from "./llm-bridge";

/**
 * One form for every OpenAI-compatible endpoint there is.
 *
 * The presets are a shortcut past typing a URL that is already known, not a
 * list of vendors Inertia blesses, so "Start blank" sits beside them with the
 * same weight. Everything after that step is identical whichever way it was
 * reached.
 */

/** Three honest states for a key, rather than a blank field meaning two things. */
const KEY_NONE = "none";
const KEY_EXISTING = "existing";
const KEY_NEW = "new";

/** OPENAI_API_KEY from "OpenAI", so the suggested name is the one people expect. */
function suggestSecretName(name) {
  const slug = String(name ?? "")
    .toUpperCase()
    .replace(/[^A-Z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .replace(/^([0-9])/, "_$1");
  return slug ? `${slug}_API_KEY` : "API_KEY";
}

const SECRET_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** Stored as an empty string, which a select cannot hold without meaning
 *  "nothing chosen" - so the automatic option gets a value of its own. */
const PROTOCOL_AUTO = "auto";

function PresetPicker({ onPick, onBlank }) {
  return (
    <div className="flex flex-col gap-2.5">
      <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
        Start from an endpoint you already know, or fill the form in yourself.
        Anything speaking the OpenAI chat-completions API works either way.
      </p>
      <div className="grid grid-cols-1 gap-1 sm:grid-cols-2">
        {PRESETS.map((preset) => (
          <button
            key={preset.name}
            type="button"
            onClick={() => onPick(preset)}
            className={cn(
              "flex items-center gap-2 rounded-lg fill-whisper px-2.5 py-2 text-left outline-none",
              "transition-colors duration-150 ease-out hover:fill-control-hover",
              "focus-visible:fill-control-hover",
            )}
          >
            <span className="flex size-6 shrink-0 items-center justify-center rounded-full fill-control text-muted-foreground">
              <Icon
                name={preset.icon}
                className="size-3.5"
                aria-hidden="true"
              />
            </span>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-xs text-foreground">
                {preset.name}
              </span>
              <span className="block truncate font-mono text-[0.625rem] text-muted-foreground">
                {preset.baseUrl}
              </span>
            </span>
          </button>
        ))}
      </div>
      <Button
        variant="subtle"
        size="xs"
        className="self-start"
        onClick={onBlank}
      >
        <Plus />
        Start blank
      </Button>
    </div>
  );
}

export function ProviderDialog({
  open,
  onOpenChange,
  initial,
  providers,
  onSave,
}) {
  const { client, configured } = useWorkspace();
  const isNew = !initial;

  const [draft, setDraft] = React.useState(blankProvider);
  const [stage, setStage] = React.useState("form");
  const [keyMode, setKeyMode] = React.useState(KEY_NONE);
  const [newSecret, setNewSecret] = React.useState({ name: "", value: "" });
  const [secretNames, setSecretNames] = React.useState([]);
  const [probe, setProbe] = React.useState(null);
  const [testing, setTesting] = React.useState(false);
  const [saving, setSaving] = React.useState(false);
  const [formError, setFormError] = React.useState(null);

  // Reset on open rather than on prop change: the dialog is mounted the whole
  // time, so a stale draft would otherwise survive a cancel.
  React.useEffect(() => {
    if (!open) return;
    const base = initial ? { ...blankProvider(), ...initial } : blankProvider();
    setDraft(base);
    setStage(initial ? "form" : "preset");
    setKeyMode(base.apiKeySecret ? KEY_EXISTING : KEY_NONE);
    setNewSecret({ name: "", value: "" });
    setProbe(null);
    setFormError(null);
  }, [open, initial]);

  React.useEffect(() => {
    if (!open || !configured) return undefined;
    let alive = true;
    client.secrets
      .list()
      .then((rows) => alive && setSecretNames(rows.map((row) => row.name)))
      .catch(() => alive && setSecretNames([]));
    return () => {
      alive = false;
    };
  }, [open, configured, client]);

  const errors = React.useMemo(
    () => validateProvider(draft, providers ?? []),
    [draft, providers],
  );

  const normalized = normalizeBaseUrl(draft.baseUrl);
  const showNormalized =
    Boolean(normalized) && normalized !== draft.baseUrl.trim();

  const secretOptions = React.useMemo(() => {
    const all = new Set(secretNames);
    if (draft.apiKeySecret) all.add(draft.apiKeySecret);
    return [
      {
        value: KEY_NONE,
        label: "No key needed",
        description: "Local runtimes rarely want one",
      },
      ...[...all].sort().map((name) => ({
        value: name,
        label: name,
        icon: KeyRound,
        description: secretNames.includes(name)
          ? undefined
          : "Not in this workspace yet",
      })),
      { value: KEY_NEW, label: "Add a new key…", icon: Plus },
    ];
  }, [secretNames, draft.apiKeySecret]);

  const selectedSecret =
    keyMode === KEY_NEW
      ? KEY_NEW
      : keyMode === KEY_EXISTING
        ? draft.apiKeySecret
        : KEY_NONE;

  function chooseSecret(next) {
    if (next === KEY_NONE) {
      setKeyMode(KEY_NONE);
      setDraft((d) => ({ ...d, apiKeySecret: "" }));
      return;
    }
    if (next === KEY_NEW) {
      setKeyMode(KEY_NEW);
      setNewSecret((s) => ({
        ...s,
        name: s.name || suggestSecretName(draft.name),
      }));
      return;
    }
    setKeyMode(KEY_EXISTING);
    setDraft((d) => ({ ...d, apiKeySecret: next }));
  }

  /** A preset whose usual secret is already in the vault should point at it;
   *  otherwise the name is only a suggestion for the key about to be typed. */
  function applyPreset(preset) {
    const known = Boolean(preset.secret) && secretNames.includes(preset.secret);
    setDraft((d) => ({
      ...d,
      name: preset.name,
      baseUrl: preset.baseUrl,
      apiKeySecret: known ? preset.secret : "",
      // Only Anthropic's preset carries one. Everything else leaves it empty,
      // which means "work it out from the base URL" and resolves to the
      // protocol every other endpoint on this list speaks.
      kind: preset.kind ?? "",
    }));
    setKeyMode(known ? KEY_EXISTING : preset.secret ? KEY_NEW : KEY_NONE);
    setNewSecret({ name: preset.secret ?? "", value: "" });
    setStage("form");
  }

  /**
   * A key that has only been typed cannot be sent anywhere, because the request
   * is built in the main process from the stored secret. So testing commits it
   * first. That is a write the user asked for by pressing the button with a key
   * in the field, and it is the only way the test can mean anything.
   */
  async function persistNewSecret() {
    const name = newSecret.name.trim();
    if (!SECRET_NAME.test(name)) {
      setFormError(
        "A secret name must start with a letter or underscore and hold only letters, digits and underscores.",
      );
      return null;
    }
    if (!newSecret.value) {
      setFormError("Paste the key, or choose No key needed.");
      return null;
    }
    await client.secrets.set(
      name,
      newSecret.value,
      `API key for ${draft.name.trim()}`,
    );
    setSecretNames((prev) => (prev.includes(name) ? prev : [...prev, name]));
    return name;
  }

  async function resolveSecretName() {
    if (keyMode === KEY_NEW) return persistNewSecret();
    if (keyMode === KEY_EXISTING) return draft.apiKeySecret || "";
    return "";
  }

  async function test() {
    setFormError(null);
    setTesting(true);
    try {
      const apiKeySecret = await resolveSecretName();
      if (apiKeySecret === null) return;
      const candidate = { ...draft, apiKeySecret };
      setDraft(candidate);
      if (apiKeySecret) {
        setKeyMode(KEY_EXISTING);
        setNewSecret({ name: "", value: "" });
      }
      const result = await testProvider(candidate);
      // The test found the endpoint speaks the other protocol from the one the
      // address suggested. Written onto the record, so chat uses what answered
      // rather than re-guessing from the address on every request.
      if (result.ok && result.detected && result.protocol) {
        setDraft((d) => ({ ...d, kind: result.protocol }));
      }
      setProbe(
        result.ok
          ? {
              ok: true,
              models: result.models ?? 0,
              latencyMs: result.latencyMs ?? 0,
              note: result.note ?? "",
              protocol: result.detected ? result.protocol : "",
            }
          : { ok: false, error: describeFailure(result) },
      );
    } catch (failure) {
      setProbe({ ok: false, error: failure?.message || "The request failed." });
    } finally {
      setTesting(false);
    }
  }

  async function save() {
    setFormError(null);
    if (Object.keys(errors).length) return;
    setSaving(true);
    try {
      const apiKeySecret = await resolveSecretName();
      if (apiKeySecret === null) return;
      await onSave({ ...draft, name: draft.name.trim(), apiKeySecret });
    } catch (failure) {
      setFormError(failure?.message || "Could not save this provider.");
    } finally {
      setSaving(false);
    }
  }

  const choosingPreset = stage === "preset";

  // What the URL alone would say, for the automatic option's label, and what
  // will actually be used once the explicit choice is taken into account.
  const detected = protocolOf({ baseUrl: draft.baseUrl });
  const resolvedProtocol = protocolOf(draft);

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      size="lg"
      ariaLabel={isNew ? "Add provider" : "Edit provider"}
    >
      <DialogTitle>
        {isNew ? "Add a provider" : `Edit ${initial?.name ?? "provider"}`}
      </DialogTitle>

      <DialogBody className="flex max-h-[60vh] flex-col gap-3 overflow-y-auto no-scrollbar">
        {/* Keyed on the stage, so picking a preset fades the form in over the
            picker rather than the picker's tiles turning into fields. */}
        <div key={stage} className="flex animate-fade-in flex-col gap-3">
        {choosingPreset ? (
          <PresetPicker onPick={applyPreset} onBlank={() => setStage("form")} />
        ) : (
          <>
            <SettingsField
              label="Name"
              htmlFor="provider-name"
              description="Yours to choose. This is only what you will see in the model picker, so two endpoints from the same vendor can be told apart."
            >
              <Input
                id="provider-name"
                size="sm"
                autoFocus
                autoComplete="off"
                placeholder="My gateway"
                value={draft.name}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, name: e.target.value }))
                }
              />
            </SettingsField>
            {errors.name ? (
              <p className="-mt-2 animate-fade-in text-[0.6875rem] leading-relaxed text-destructive-ink">
                {errors.name}
              </p>
            ) : null}

            <SettingsField
              label="Base URL"
              htmlFor="provider-url"
              description={
                showNormalized
                  ? `Requests will go to ${normalized}`
                  : "The root of the OpenAI-compatible API. A /v1 is added only when the URL has no path of its own."
              }
            >
              <Input
                id="provider-url"
                size="sm"
                spellCheck={false}
                autoComplete="off"
                className="font-mono"
                placeholder="https://api.example.com/v1"
                value={draft.baseUrl}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, baseUrl: e.target.value }))
                }
              />
            </SettingsField>
            {errors.baseUrl ? (
              <p className="-mt-2 animate-fade-in text-[0.6875rem] leading-relaxed text-destructive-ink">
                {errors.baseUrl}
              </p>
            ) : null}

            <SettingsField
              label="Protocol"
              htmlFor="provider-protocol"
              description={
                resolvedProtocol === "anthropic"
                  ? "Spoken natively, which is what buys prompt caching and extended thinking. On a long turn the cache is most of the difference in the bill."
                  : "The chat-completions API, which is what almost every endpoint speaks. Set this yourself only for a proxy that answers Anthropic's own API from an address that does not say so."
              }
            >
              <Select
                id="provider-protocol"
                size="sm"
                value={draft.kind || PROTOCOL_AUTO}
                onChange={(next) =>
                  setDraft((d) => ({
                    ...d,
                    kind: next === PROTOCOL_AUTO ? "" : next,
                  }))
                }
                options={[
                  {
                    value: PROTOCOL_AUTO,
                    label: "Detect from the address",
                    description: PROTOCOL_LABELS[detected],
                  },
                  ...PROTOCOLS.map((kind) => ({
                    value: kind,
                    label: PROTOCOL_LABELS[kind],
                  })),
                ]}
              />
            </SettingsField>

            <SettingsField
              label="API key"
              description="Stored as a named secret in your workspace, never on the provider record itself. Local runtimes such as Ollama do not need one."
            >
              <Select
                size="xs"
                ariaLabel="API key secret"
                value={selectedSecret}
                onChange={chooseSecret}
                options={secretOptions}
              />
            </SettingsField>

            <Collapse open={keyMode === KEY_NEW}>
              <div className="flex flex-col gap-2 rounded-lg fill-whisper p-2.5">
                <Input
                  size="xs"
                  className="font-mono"
                  spellCheck={false}
                  autoComplete="off"
                  placeholder="OPENAI_API_KEY"
                  aria-label="Secret name"
                  value={newSecret.name}
                  onChange={(e) =>
                    setNewSecret((s) => ({ ...s, name: e.target.value }))
                  }
                />
                <Input
                  size="xs"
                  type="password"
                  className="font-mono"
                  spellCheck={false}
                  autoComplete="off"
                  placeholder="Paste the key"
                  aria-label="API key"
                  value={newSecret.value}
                  onChange={(e) =>
                    setNewSecret((s) => ({ ...s, value: e.target.value }))
                  }
                />
                <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
                  Saved under this name when you save the provider or test the
                  connection, then referred to by name from then on.
                </p>
              </div>
            </Collapse>

            <SettingsField
              label="Custom headers"
              description="Optional. Some gateways want a routing or attribution header alongside the key."
            >
              <KeyValueRows
                value={draft.headers}
                onChange={(headers) => setDraft((d) => ({ ...d, headers }))}
              />
            </SettingsField>

            <SettingsField label="Models">
              <ModelsField
                draft={draft}
                value={draft.models}
                onChange={(models) => setDraft((d) => ({ ...d, models }))}
              />
            </SettingsField>

            <div className="flex flex-col gap-2 rounded-lg fill-whisper p-2.5">
              <div className="flex items-center gap-2">
                <Button
                  variant="subtle"
                  size="xs"
                  disabled={testing || Boolean(errors.baseUrl)}
                  onClick={test}
                >
                  {testing ? <Spinner size="sm" /> : <Plug />}
                  {testing ? "Testing…" : "Test connection"}
                </Button>
                {probe?.ok ? (
                  <span className="flex min-w-0 animate-fade-in items-center gap-1.5 text-[0.6875rem] text-success-ink">
                    <CircleCheck
                      className="size-3.5 shrink-0"
                      aria-hidden="true"
                    />
                    {probe.note
                      ? `Reachable in ${probe.latencyMs}ms. ${probe.note}`
                      : `${probe.models} models in ${probe.latencyMs}ms`}
                    {probe.protocol
                      ? ` Detected ${PROTOCOL_LABELS[probe.protocol] ?? probe.protocol}; the protocol above was set to match.`
                      : ""}
                  </span>
                ) : null}
              </div>
              {probe && !probe.ok ? (
                <p className="flex animate-fade-in items-start gap-1.5 text-[0.6875rem] leading-relaxed text-destructive-ink">
                  <CircleAlert
                    className="mt-px size-3.5 shrink-0"
                    aria-hidden="true"
                  />
                  <span>{probe.error}</span>
                </p>
              ) : null}
            </div>

            {formError ? (
              <p className="animate-fade-in text-[0.6875rem] leading-relaxed text-destructive-ink">
                {formError}
              </p>
            ) : null}
          </>
        )}
        </div>
      </DialogBody>

      <DialogFooter>
        <Button
          variant="secondary"
          size="pill"
          onClick={() => onOpenChange(false)}
        >
          Cancel
        </Button>
        {choosingPreset ? null : (
          <Button
            variant="primary"
            size="pill"
            disabled={saving || Object.keys(errors).length > 0}
            onClick={save}
          >
            {saving ? <Spinner size="sm" /> : null}
            {isNew ? "Add provider" : "Save"}
          </Button>
        )}
      </DialogFooter>
    </Dialog>
  );
}

/* -- headers ----------------------------------------------------------- */

let rowSeq = 0;
const nextRowId = () => `hdr-${(rowSeq += 1)}`;

/**
 * A local twin of the integrations key/value editor rather than an import
 * across features: this one sits in a 28px settings pane and that one is built
 * for the 32px plugin editors, and the two densities do not mix on one screen.
 * Rows are held as a list because an object cannot carry a half-typed key.
 */
function KeyValueRows({ value, onChange }) {
  const [rows, setRows] = React.useState(() =>
    Object.entries(value ?? {}).map(([key, val]) => ({
      id: nextRowId(),
      key,
      value: String(val ?? ""),
    })),
  );

  function commit(next) {
    setRows(next);
    const out = {};
    for (const row of next) {
      const key = row.key.trim();
      if (key) out[key] = row.value;
    }
    onChange?.(out);
  }

  return (
    <div className="flex flex-col gap-1.5">
      {rows.map((row, index) => (
        <div key={row.id} className="flex items-center gap-1.5">
          <Input
            size="xs"
            className="flex-1 font-mono"
            placeholder="Header"
            aria-label={`Header name ${index + 1}`}
            value={row.key}
            onChange={(e) =>
              commit(
                rows.map((r) =>
                  r.id === row.id ? { ...r, key: e.target.value } : r,
                ),
              )
            }
          />
          <Input
            size="xs"
            className="flex-1 font-mono"
            placeholder="value"
            aria-label={`Header value ${index + 1}`}
            value={row.value}
            onChange={(e) =>
              commit(
                rows.map((r) =>
                  r.id === row.id ? { ...r, value: e.target.value } : r,
                ),
              )
            }
          />
          <Button
            variant="ghost"
            size="xs"
            onClick={() => commit(rows.filter((r) => r.id !== row.id))}
          >
            Remove
          </Button>
        </div>
      ))}
      <Button
        variant="subtle"
        size="xs"
        className="self-start"
        onClick={() =>
          commit([...rows, { id: nextRowId(), key: "", value: "" }])
        }
      >
        <Plus />
        Add header
      </Button>
    </div>
  );
}
