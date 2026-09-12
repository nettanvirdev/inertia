import * as React from "react";
import {
  CircleAlert,
  Pencil,
  Plug,
  Plus,
  RotateCw,
  Server,
  Trash2,
} from "@/components/icons";
import { useWorkspace } from "@/lib/workspace";
import { allModels, normalizeBaseUrl, providerId } from "@shared/providers";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { IconButton } from "@/components/ui/icon-button";
import { Select } from "@/components/ui/select";
import { SkeletonRow } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Tooltip } from "@/components/ui/tooltip";
import { useToast } from "@/components/ui/toast";
import { SettingsCard, SettingsRow, SettingsSection } from "../SettingsRow";
import { ProviderDialog } from "./models/ProviderDialog";
import { describeFailure, testProvider } from "./models/llm-bridge";

/**
 * Providers, not vendors.
 *
 * Everything Inertia can talk to speaks one protocol, so this pane manages a
 * list of endpoints the user named themselves rather than a fixed set of
 * brands with a key slot each. The document is the source of truth and every
 * change round-trips through it: a list that updated optimistically and then
 * failed would be the UI claiming something that is not on disk.
 */

const DOC = "settings.models";
const EMPTY = { providers: [], defaultModel: "" };

/** Every model on every enabled provider, grouped the way a picker reads. */
function modelOptions(providers) {
  const groups = new Map();
  for (const model of allModels(providers)) {
    const key = model.providerName || model.providerId;
    if (!groups.has(key)) groups.set(key, []);
    groups
      .get(key)
      .push({ value: model.ref, label: model.label, description: model.id });
  }
  return [...groups].map(([group, options]) => ({ group, options }));
}

function ConnectionBadge({ probe }) {
  if (!probe) return <Badge size="sm">Not tested</Badge>;
  if (probe.state === "testing") return <Badge size="sm">Testing…</Badge>;
  if (probe.state === "ok") {
    return (
      <Badge size="sm" variant="success" dot>
        {probe.latencyMs}ms
      </Badge>
    );
  }
  return (
    <Badge size="sm" variant="danger" dot>
      Unreachable
    </Badge>
  );
}

function ProviderRow({ provider, probe, onToggle, onTest, onEdit, onRemove }) {
  const count = provider.models?.length ?? 0;
  return (
    <div className="flex animate-slide-up flex-col gap-1 rounded-lg px-2 py-1.5 transition-colors duration-150 ease-out hover:fill-control-hover">
      <div className="flex min-w-0 items-center gap-2">
        <Server
          className="size-3.5 shrink-0 text-muted-foreground"
          aria-hidden="true"
        />

        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="truncate text-xs text-foreground">
              {provider.name}
            </span>
            <ConnectionBadge probe={probe} />
          </div>
          <p className="truncate font-mono text-[0.6875rem] leading-tight text-muted-foreground">
            {normalizeBaseUrl(provider.baseUrl)}
          </p>
        </div>

        <span className="hidden shrink-0 text-[0.6875rem] text-muted-foreground sm:block">
          {count === 1 ? "1 model" : `${count} models`}
        </span>

        <Tooltip content="Test connection">
          <IconButton
            size="sm"
            label={`Test ${provider.name}`}
            onClick={onTest}
          >
            {probe?.state === "testing" ? <Spinner size="sm" /> : <RotateCw />}
          </IconButton>
        </Tooltip>
        <Tooltip content="Edit">
          <IconButton
            size="sm"
            label={`Edit ${provider.name}`}
            onClick={onEdit}
          >
            <Pencil />
          </IconButton>
        </Tooltip>
        <Tooltip content="Remove">
          <IconButton
            size="sm"
            label={`Remove ${provider.name}`}
            onClick={onRemove}
          >
            <Trash2 />
          </IconButton>
        </Tooltip>
        <Switch
          size="sm"
          className="ml-0.5"
          label={`Enable ${provider.name}`}
          checked={provider.enabled !== false}
          onCheckedChange={onToggle}
        />
      </div>

      <Collapse open={probe?.state === "error"}>
        <p className="flex items-start gap-1.5 pl-5 text-[0.6875rem] leading-relaxed text-destructive-ink">
          <CircleAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
          <span>{probe?.error}</span>
        </p>
      </Collapse>
    </div>
  );
}

export function ModelsPane() {
  const { client, configured } = useWorkspace();
  const { toast } = useToast();

  const [doc, setDoc] = React.useState(EMPTY);
  const [loading, setLoading] = React.useState(configured);
  const [loadError, setLoadError] = React.useState(null);

  // Reachability is a fact about right now, not about the record, so it lives
  // in memory and starts blank every time the pane is opened.
  const [probes, setProbes] = React.useState({});

  const [editing, setEditing] = React.useState(null); // { provider } or { provider: null }
  const [removing, setRemoving] = React.useState(null);

  React.useEffect(() => {
    if (!configured) {
      setLoading(false);
      return undefined;
    }
    let alive = true;
    setLoading(true);
    client
      .readDocument(DOC, EMPTY)
      .then((value) => {
        if (!alive) return;
        setDoc({ ...EMPTY, ...value, providers: value?.providers ?? [] });
        setLoadError(null);
      })
      .catch((failure) => alive && setLoadError(failure.message))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [client, configured]);

  const providers = doc.providers ?? [];
  const options = React.useMemo(() => modelOptions(providers), [providers]);

  const commit = React.useCallback(
    async (next) => {
      const merged = { ...doc, ...next };
      await client.writeDocument(DOC, merged);
      setDoc(merged);
      return merged;
    },
    [client, doc],
  );

  async function guarded(work, message) {
    try {
      await work();
    } catch (failure) {
      toast({
        title: message,
        description: failure.message,
        variant: "danger",
      });
    }
  }

  async function saveProvider(draft) {
    const taken = providers.filter((p) => p.id !== draft.id).map((p) => p.id);
    const record = { ...draft, id: draft.id || providerId(draft.name, taken) };
    const exists = providers.some((p) => p.id === record.id);
    await commit({
      providers: exists
        ? providers.map((p) => (p.id === record.id ? record : p))
        : [...providers, record],
    });
    setEditing(null);
    toast({
      title: exists ? `${record.name} saved` : `${record.name} added`,
      description: record.models?.length
        ? "Its models are selectable wherever a model is chosen."
        : "Add a model to it and it will show up in the model picker.",
      variant: "success",
    });
  }

  async function removeProvider() {
    if (!removing) return;
    const rest = providers.filter((p) => p.id !== removing.id);
    // A default pointing at a provider that is gone is worse than no default:
    // it silently resolves to nothing at the moment a message is sent.
    const stillThere = allModels(rest).some((m) => m.ref === doc.defaultModel);
    await guarded(async () => {
      await commit({
        providers: rest,
        defaultModel: stillThere ? doc.defaultModel : "",
      });
      setRemoving(null);
    }, "Could not remove that provider");
  }

  async function runTest(provider) {
    setProbes((prev) => ({ ...prev, [provider.id]: { state: "testing" } }));
    const result = await testProvider(provider);
    setProbes((prev) => ({
      ...prev,
      [provider.id]: result.ok
        ? {
            state: "ok",
            latencyMs: result.latencyMs ?? 0,
            models: result.models ?? 0,
          }
        : { state: "error", error: describeFailure(result) },
    }));
  }

  if (!configured) {
    return (
      <EmptyState
        icon={Server}
        title="No workspace folder yet"
        description="Providers are saved in the workspace folder. Choose one in Workspace settings and this pane fills in."
      />
    );
  }

  return (
    <div className="w-full">
      <SettingsSection
        title="Default model"
        description="Used wherever an agent has not pinned a model of its own."
      >
        {providers.length === 0 ? (
          <p className="px-4 py-3 text-[0.6875rem] leading-relaxed text-muted-foreground">
            Nothing to choose from yet. Add a provider below and its models land
            here.
          </p>
        ) : (
          <SettingsRow
            label="Model"
            description={
              options.length
                ? "Grouped by the provider you named."
                : "None of your enabled providers has a model on it yet."
            }
            control={
              <Select
                size="xs"
                className="w-64"
                panelClassName="w-72"
                ariaLabel="Default model"
                placeholder={
                  options.length ? "Select a model" : "No models available"
                }
                disabled={!options.length}
                value={doc.defaultModel ?? ""}
                onChange={(value) =>
                  guarded(
                    () => commit({ defaultModel: value }),
                    "Could not save the default model",
                  )
                }
                options={options}
              />
            }
          />
        )}
      </SettingsSection>

      <SettingsSection flat
        title="Your providers"
        description="Any endpoint that speaks the OpenAI chat-completions API. Keys are stored as named secrets in your workspace, and requests are made from the app itself rather than the window."
      >
        <SettingsCard className="flex flex-col gap-0.5 p-1.5">
          {loading ? (
            <SkeletonRow lines={3} className="p-2" />
          ) : loadError ? (
            <p className="p-2 text-[0.6875rem] leading-relaxed text-muted-foreground">
              {loadError}
            </p>
          ) : providers.length ? (
            providers.map((provider) => (
              <ProviderRow
                key={provider.id}
                provider={provider}
                probe={probes[provider.id]}
                onTest={() => runTest(provider)}
                onEdit={() => setEditing({ provider })}
                onRemove={() => setRemoving(provider)}
                onToggle={(enabled) =>
                  guarded(
                    () =>
                      commit({
                        providers: providers.map((p) =>
                          p.id === provider.id ? { ...p, enabled } : p,
                        ),
                      }),
                    "Could not change that provider",
                  )
                }
              />
            ))
          ) : (
            <EmptyState
              className="py-8"
              icon={Plug}
              title="No providers yet"
              description="Add any OpenAI-compatible endpoint - a hosted API, a gateway, or a model running on this machine. You give it the name you will see."
            />
          )}
        </SettingsCard>

        <div className="flex">
          <Button
            variant="primary"
            size="xs"
            onClick={() => setEditing({ provider: null })}
          >
            <Plus />
            Add provider
          </Button>
        </div>
      </SettingsSection>

      <ProviderDialog
        open={Boolean(editing)}
        onOpenChange={(open) => !open && setEditing(null)}
        initial={editing?.provider ?? null}
        providers={providers}
        onSave={saveProvider}
      />

      <ConfirmDialog
        open={Boolean(removing)}
        onOpenChange={(open) => !open && setRemoving(null)}
        destructive
        title={`Remove ${removing?.name ?? "this provider"}?`}
        description="Its models stop being selectable everywhere in Inertia. The API key stays in your secrets - remove it separately if you are done with it."
        confirmLabel="Remove provider"
        onConfirm={removeProvider}
      />
    </div>
  );
}
