import * as React from "react";
import { CircleCheck, CircleX, ExternalLink, Play } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useWorkspace } from "@/lib/workspace";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { SkeletonRow } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Tooltip } from "@/components/ui/tooltip";
import { useToast } from "@/components/ui/toast";
import { SettingsCard, SettingsSection } from "../../SettingsRow";
import { SecretDialog } from "./SecretDialog";
import { SecretRow } from "./SecretRow";
import { REQUIRED_SECRETS } from "./required";

/**
 * The keys the app needs by name, as rows.
 *
 * The Composio key used to be a card the height of six rows: a paste field, a
 * Save button, a paragraph explaining that the stored key is never shown, and a
 * row of buttons. It was the only key in the pane that worked differently from
 * every other key in the pane, and it was the one people asked about. It reads
 * as a row now, exactly like the ones underneath, and gains a reveal it never
 * had. What it keeps from the card is the Test button, which fits.
 *
 * No rename, no delete. Renaming would disconnect the feature, since the main
 * process looks the key up by the string in the catalogue; deleting is a
 * decision worth making on the screen that owns the feature rather than in a
 * list of names.
 */
function openExternal(url) {
  if (window.electronAPI?.openExternal) window.electronAPI.openExternal(url);
  else window.open(url, "_blank", "noopener,noreferrer");
}

export function RequiredKeys({ entries, loading, onChanged }) {
  const { client } = useWorkspace();
  const { toast } = useToast();

  const [revealed, setRevealed] = React.useState({});
  const [reports, setReports] = React.useState({});
  const [testing, setTesting] = React.useState(null);
  const [editing, setEditing] = React.useState(null); // { spec, value }
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState(null);

  const stored = React.useMemo(() => {
    const map = new Map();
    for (const entry of entries ?? []) map.set(entry.name, entry);
    return map;
  }, [entries]);

  async function toggleReveal(name) {
    if (revealed[name] !== undefined) {
      setRevealed((prev) => {
        const next = { ...prev };
        delete next[name];
        return next;
      });
      return;
    }
    try {
      const value = (await client.secrets.get(name)) ?? "";
      setRevealed((prev) => ({ ...prev, [name]: value }));
    } catch (failure) {
      toast({ variant: "danger", title: "Could not read that secret", description: failure.message });
    }
  }

  async function test(spec) {
    setTesting(spec.name);
    setReports((prev) => ({ ...prev, [spec.name]: null }));
    try {
      const report = await spec.test();
      setReports((prev) => ({ ...prev, [spec.name]: report }));
    } catch (failure) {
      setReports((prev) => ({ ...prev, [spec.name]: { ok: false, message: failure.message } }));
    } finally {
      setTesting(null);
    }
  }

  async function save() {
    if (!editing) return;
    const key = String(editing.value ?? "").trim();
    if (!key) {
      setError("Paste the key first.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await client.secrets.set(editing.spec.name, key, editing.spec.label);
      // A key that changed makes an open reveal a lie, and makes any earlier
      // test result a claim about a key that is no longer stored.
      setRevealed((prev) => {
        const next = { ...prev };
        delete next[editing.spec.name];
        return next;
      });
      setReports((prev) => ({ ...prev, [editing.spec.name]: null }));
      setEditing(null);
      await onChanged?.();
      toast({ variant: "success", title: `${editing.spec.title} key saved` });
    } catch (failure) {
      setError(failure.message);
    } finally {
      setSaving(false);
    }
  }

  return (
    <SettingsSection flat
      title="Keys Inertia needs"
      description="Two features authenticate with a key of their own, under the exact names below. They cannot be renamed, because the app looks them up by name before you have named anything. Test one to prove it works rather than that something is stored."
    >
      <SettingsCard className="flex flex-col gap-0.5 p-1.5">
        {loading ? (
          <SkeletonRow lines={2} className="p-2" />
        ) : (
          REQUIRED_SECRETS.map((spec) => {
            const entry = stored.get(spec.name) ?? null;
            const report = reports[spec.name];
            return (
              <SecretRow
                key={spec.name}
                entry={entry}
                name={spec.name}
                icon={spec.icon}
                purpose={spec.purpose}
                note={report ? report.message : undefined}
                revealed={revealed[spec.name] !== undefined}
                value={revealed[spec.name]}
                onToggle={() => toggleReveal(spec.name)}
                onCopy={() => client.secrets.get(spec.name)}
                onEdit={() => {
                  setError(null);
                  setEditing({ spec, value: "" });
                }}
                actions={
                  <>
                    {report ? (
                      <span
                        className={cn(
                          "shrink-0 animate-pop-in",
                          report.ok ? "text-success-ink" : "text-destructive-ink"
                        )}
                        aria-hidden="true"
                      >
                        {report.ok ? (
                          <CircleCheck className="size-3.5" />
                        ) : (
                          <CircleX className="size-3.5" />
                        )}
                      </span>
                    ) : null}
                    {entry ? (
                      <Tooltip content="Test the key">
                        <IconButton
                          size="sm"
                          label={`Test ${spec.name}`}
                          disabled={testing === spec.name}
                          onClick={() => test(spec)}
                        >
                          {testing === spec.name ? <Spinner size="sm" /> : <Play />}
                        </IconButton>
                      </Tooltip>
                    ) : (
                      <Button
                        size="xs"
                        variant="subtle"
                        className="shrink-0"
                        onClick={() => openExternal(spec.url)}
                      >
                        <ExternalLink />
                        {spec.urlLabel}
                      </Button>
                    )}
                  </>
                }
              />
            );
          })
        )}
      </SettingsCard>

      <SecretDialog
        open={Boolean(editing)}
        onOpenChange={(open) => {
          if (!open) {
            setEditing(null);
            setError(null);
          }
        }}
        editing={
          editing
            ? {
                name: editing.spec.name,
                value: editing.value,
                label: editing.spec.label,
                isNew: !stored.has(editing.spec.name),
              }
            : null
        }
        onChange={(patch) => setEditing((prev) => ({ ...prev, value: patch.value ?? prev.value }))}
        onSave={save}
        saving={saving}
        error={error}
        title={
          editing
            ? `${stored.has(editing.spec.name) ? "Replace" : "Add"} the ${editing.spec.title} key`
            : undefined
        }
        confirmLabel={editing && stored.has(editing.spec.name) ? "Replace" : "Save key"}
      />
    </SettingsSection>
  );
}
