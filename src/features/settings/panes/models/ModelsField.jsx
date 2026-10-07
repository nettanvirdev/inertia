import * as React from "react";
import { CircleAlert, Download, Plus, Trash2, X } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapse } from "@/components/ui/collapse";
import { Input, SearchInput } from "@/components/ui/input";
import { IconButton } from "@/components/ui/icon-button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { describeFailure, listModels } from "./llm-bridge";
import { knownContextWindow } from "@shared/models";
import { CACHE_READ_MULTIPLIER, CACHE_WRITE_MULTIPLIER } from "@shared/usage";

/**
 * The model list for one provider.
 *
 * Two ways in, because neither one is enough on its own. `GET /models` is the
 * fast path and most endpoints implement it, but plenty of gateways return a
 * catalogue of hundreds and some return a wrong list or none at all. So the
 * fetched catalogue is always searchable and never the only way to name a
 * model - a typed id is a first-class answer, not a fallback for errors.
 */

/**
 * Kept alongside the id: the display name, and the context window.
 *
 * The window matters more than it looks. It is the number the conversation is
 * compacted against, and this used to drop it on the floor - a model picked
 * from a catalogue that said 262,144 was saved as an id and a label, and the
 * app then assumed 32k for it, summarising a conversation that had eight
 * times the room. What the endpoint reports comes through; what the user
 * types wins over it.
 */
function normalize(models) {
  return (models ?? []).map((model) =>
    typeof model === "string"
      ? { id: model, label: "", context: null, input: null, output: null }
      : {
          id: model.id,
          label: model.label ?? "",
          context: asWindow(model.context),
          input: asPrice(model.input),
          output: asPrice(model.output),
          cachedInput: asPrice(model.cachedInput),
          cacheWrite: asPrice(model.cacheWrite),
        }
  );
}

/** A context window is a whole number of tokens, or nothing. */
function asWindow(value) {
  const n = Number(String(value ?? "").replace(/[,_\s]/g, ""));
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : null;
}

/** A price is USD per million tokens - a non-negative number, or nothing. A
 *  zero is kept, because a local model that costs nothing is a real answer. */
function asPrice(value) {
  if (value === "" || value == null) return null;
  const n = Number(String(value).replace(/[$,\s]/g, ""));
  return Number.isFinite(n) && n >= 0 ? n : null;
}

/** 131072 reads as "131k"; the field is a small one. */
function shortWindow(n) {
  if (!n) return "";
  return n >= 1000 ? `${Math.round(n / 1000)}k` : String(n);
}

/**
 * The window a model gets when nobody has typed one: the shipped table's
 * answer, or the cautious default the compactor falls back to.
 */
function assumedWindow(id) {
  const known = knownContextWindow(id);
  return known ? `${shortWindow(known)} known` : "32k assumed";
}

/**
 * A price field, in dollars per million tokens.
 *
 * A `$` sits inside the field and the unit sits after it, because "0.15" with
 * no anchor is a number the reader has to guess the meaning of - per token, per
 * thousand, per call. The value is held as typed while the field has focus so a
 * half-typed "0." is not snapped to a number mid-keystroke; it settles on blur.
 */
function PriceInput({ label, value, onChange, hint }) {
  const [text, setText] = React.useState(value == null ? "" : String(value));
  // While the field has focus its own text is the truth, so a half-typed "0."
  // is not snapped to a number and echoed back mid-keystroke. Between edits it
  // follows the stored value, so a reset or a load shows through.
  const focused = React.useRef(false);

  React.useEffect(() => {
    if (!focused.current) setText(value == null ? "" : String(value));
  }, [value]);

  return (
    <span className="flex items-center gap-1">
      <Input
        size="xs"
        className="w-20 shrink-0 tabular-nums"
        inputMode="decimal"
        aria-label={label}
        title={label}
        leadingIcon={<span className="text-[0.625rem] text-muted-foreground">$</span>}
        value={text}
        onFocus={() => {
          focused.current = true;
        }}
        onChange={(e) => {
          setText(e.target.value);
          onChange(asPrice(e.target.value));
        }}
        onBlur={() => {
          focused.current = false;
          setText(value == null ? "" : String(value));
        }}
      />
      <span className="text-[0.625rem] text-muted-foreground">{hint}</span>
    </span>
  );
}

function CatalogRow({ entry, checked, onToggle }) {
  return (
    <button
      type="button"
      onClick={onToggle}
      className={cn(
        "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left outline-none",
        "transition-colors duration-150 ease-out hover:fill-control-hover",
        "focus-visible:fill-control-hover"
      )}
    >
      <Checkbox size="sm" checked={checked} label={entry.id} />
      <span className="min-w-0 flex-1 truncate font-mono text-[0.6875rem] text-foreground">
        {entry.id}
      </span>
      {entry.label && entry.label !== entry.id ? (
        <span className="hidden max-w-[12rem] shrink-0 truncate text-[0.6875rem] text-muted-foreground sm:block">
          {entry.label}
        </span>
      ) : null}
      {entry.context ? (
        <span className="shrink-0 text-[0.6875rem] tabular-nums text-muted-foreground">
          {shortWindow(entry.context)}
        </span>
      ) : null}
    </button>
  );
}

export function ModelsField({ draft, value, onChange }) {
  const models = React.useMemo(() => normalize(value), [value]);

  const [catalog, setCatalog] = React.useState(null);
  const [fetching, setFetching] = React.useState(false);
  const [error, setError] = React.useState(null);
  const [query, setQuery] = React.useState("");
  const [manual, setManual] = React.useState("");
  // Which models have their cached rates showing. Most people never set these,
  // so the two extra fields stay folded away rather than doubling every row.
  const [cachedOpen, setCachedOpen] = React.useState(() => new Set());

  const toggleCached = (id) =>
    setCachedOpen((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const selected = React.useMemo(() => new Set(models.map((m) => m.id)), [models]);

  const shown = React.useMemo(() => {
    if (!catalog) return [];
    const needle = query.trim().toLowerCase();
    if (!needle) return catalog;
    return catalog.filter(
      (entry) =>
        entry.id.toLowerCase().includes(needle) ||
        String(entry.label ?? "")
          .toLowerCase()
          .includes(needle)
    );
  }, [catalog, query]);

  async function fetchCatalog() {
    setFetching(true);
    setError(null);
    const result = await listModels(draft);
    setFetching(false);
    if (!result.ok) {
      setCatalog(null);
      setError(describeFailure(result));
      return;
    }
    setCatalog(normalize(result.models));
  }

  function toggle(entry) {
    if (selected.has(entry.id)) {
      onChange(models.filter((m) => m.id !== entry.id));
      return;
    }
    onChange([
      ...models,
      {
        id: entry.id,
        label: "",
        context: asWindow(entry.context),
        input: null,
        output: null,
        cachedInput: null,
        cacheWrite: null,
      },
    ]);
  }

  function addManual() {
    const id = manual.trim();
    if (!id || selected.has(id)) {
      setManual("");
      return;
    }
    onChange([
      ...models,
      {
        id,
        label: "",
        context: null,
        input: null,
        output: null,
        cachedInput: null,
        cacheWrite: null,
      },
    ]);
    setManual("");
  }

  return (
    <div className="flex flex-col gap-2.5">
      {/* chosen models first: this is the list that actually gets saved, and it
          stays visible while the catalogue below scrolls */}
      {models.length ? (
        <div className="flex flex-col gap-1.5 rounded-lg fill-whisper p-1.5">
          {models.map((model, index) => {
            const patch = (fields) =>
              onChange(models.map((m, i) => (i === index ? { ...m, ...fields } : m)));
            return (
              <div
                key={model.id}
                className="flex animate-slide-up flex-col gap-1 rounded-md fill-control/40 p-1.5"
              >
                <div className="flex items-center gap-1.5">
                  <span className="min-w-0 flex-1 truncate font-mono text-[0.6875rem] text-foreground">
                    {model.id}
                  </span>
                  <Input
                    size="xs"
                    className="w-36 shrink-0"
                    placeholder="Display name (optional)"
                    aria-label={`Display name for ${model.id}`}
                    value={model.label}
                    onChange={(e) => patch({ label: e.target.value })}
                  />
                  <IconButton
                    size="sm"
                    label={`Remove ${model.id}`}
                    onClick={() => onChange(models.filter((m) => m.id !== model.id))}
                  >
                    <Trash2 />
                  </IconButton>
                </div>
                {/* Context window and price sit together on their own line: both
                    are numbers the endpoint will not tell us and both are worth
                    typing once. Price is USD per million tokens - the unit every
                    provider quotes - so the session panel can show a cost rather
                    than "No price". */}
                <div className="flex flex-wrap items-center gap-1.5">
                  <Input
                    size="xs"
                    className="w-24 shrink-0 tabular-nums"
                    inputMode="numeric"
                    placeholder={assumedWindow(model.id)}
                    aria-label={`Context window for ${model.id}, in tokens`}
                    title="Context window, in tokens. Leave blank to use what the endpoint reported or the shipped table."
                    value={model.context ?? ""}
                    onChange={(e) => patch({ context: asWindow(e.target.value) })}
                  />
                  <PriceInput
                    label={`Input price for ${model.id}, USD per million tokens`}
                    value={model.input}
                    onChange={(value) => patch({ input: value })}
                    hint="in $/M"
                  />
                  <PriceInput
                    label={`Output price for ${model.id}, USD per million tokens`}
                    value={model.output}
                    onChange={(value) => patch({ output: value })}
                    hint="out $/M"
                  />
                  <button
                    type="button"
                    onClick={() => toggleCached(model.id)}
                    className={cn(
                      "rounded-md px-1.5 py-0.5 text-[0.625rem] text-muted-foreground outline-none",
                      "transition-colors duration-150 ease-out hover:text-foreground",
                      "focus-visible:fill-control-hover"
                    )}
                  >
                    {cachedOpen.has(model.id) ? "Hide cached rates" : "Cached rates…"}
                  </button>
                </div>

                {/* Caching is where a long conversation's bill actually comes
                    from, and the discount is not the same everywhere: a tenth
                    to read is the common shape, but Anthropic bills a cache
                    WRITE above the base rate, and a gateway on a negotiated
                    contract charges whatever it agreed. Left blank these fall
                    back to the usual ratios; typed, they are used exactly. */}
                <Collapse open={cachedOpen.has(model.id)}>
                  <div className="flex flex-wrap items-center gap-1.5 pt-0.5">
                    <PriceInput
                      label={`Cached input read price for ${model.id}, USD per million tokens`}
                      value={model.cachedInput}
                      onChange={(value) => patch({ cachedInput: value })}
                      hint="cached read $/M"
                    />
                    <PriceInput
                      label={`Cache write price for ${model.id}, USD per million tokens`}
                      value={model.cacheWrite}
                      onChange={(value) => patch({ cacheWrite: value })}
                      hint="cache write $/M"
                    />
                    <span className="text-[0.625rem] text-muted-foreground">
                      blank = {CACHE_READ_MULTIPLIER}× and {CACHE_WRITE_MULTIPLIER}× input
                    </span>
                  </div>
                </Collapse>
              </div>
            );
          })}
        </div>
      ) : (
        <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
          No models yet. Fetch the list from the endpoint, or type an id if you already know it.
        </p>
      )}

      <div className="flex items-center gap-1.5">
        <Button variant="subtle" size="xs" disabled={fetching} onClick={fetchCatalog}>
          {fetching ? <Spinner size="sm" /> : <Download />}
          {fetching ? "Fetching…" : catalog ? "Fetch again" : "Fetch models"}
        </Button>
        {catalog ? (
          <Badge size="sm" variant="neutral">
            {catalog.length} available
          </Badge>
        ) : null}
      </div>

      {error ? (
        <p className="flex animate-fade-in items-start gap-1.5 text-[0.6875rem] leading-relaxed text-destructive-ink">
          <CircleAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
          <span>{error}</span>
        </p>
      ) : null}

      {catalog ? (
        <div className="flex animate-fade-in flex-col gap-1.5">
          {/* a catalogue of several hundred is the normal case on a gateway, so
              search is the primary control here rather than a convenience */}
          <SearchInput
            size="xs"
            placeholder="Search models"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onClear={() => setQuery("")}
          />
          <ScrollArea fade className="max-h-52 rounded-lg fill-whisper p-1">
            {shown.length ? (
              shown.map((entry) => (
                <CatalogRow
                  key={entry.id}
                  entry={entry}
                  checked={selected.has(entry.id)}
                  onToggle={() => toggle(entry)}
                />
              ))
            ) : (
              <p className="px-2 py-3 text-[0.6875rem] text-muted-foreground">
                Nothing matches {`"${query}"`}.
              </p>
            )}
          </ScrollArea>
        </div>
      ) : null}

      <div className="flex items-center gap-1.5">
        <Input
          size="xs"
          className="flex-1 font-mono"
          spellCheck={false}
          autoComplete="off"
          placeholder="Add a model id by hand"
          aria-label="Model id"
          value={manual}
          onChange={(e) => setManual(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter") return;
            e.preventDefault();
            addManual();
          }}
          trailingSlot={
            manual ? (
              <IconButton size="sm" label="Clear" onClick={() => setManual("")}>
                <X />
              </IconButton>
            ) : null
          }
        />
        <Button variant="subtle" size="xs" disabled={!manual.trim()} onClick={addManual}>
          <Plus />
          Add
        </Button>
      </div>
    </div>
  );
}
