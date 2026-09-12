import * as React from "react";
import { KeyRound, Plus } from "@/components/icons";
import { useWorkspace } from "@/lib/workspace";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { CopyButton } from "@/components/ui/copy-button";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonRow } from "@/components/ui/skeleton";
import { useToast } from "@/components/ui/toast";
import { SettingsCard, SettingsSection } from "../SettingsRow";
import { RequiredKeys } from "./secrets/RequiredKeys";
import { SecretDialog } from "./secrets/SecretDialog";
import { SecretRow } from "./secrets/SecretRow";
import { REQUIRED_NAMES } from "./secrets/required";

/** Mirrors the rule in src/main/workspace/secrets.cjs, which is the one enforcing it. */
const NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

const EXAMPLE = "{secret:GITHUB_TOKEN}";

export function SecretsPane() {
  const { client, configured } = useWorkspace();
  const { toast } = useToast();

  const [entries, setEntries] = React.useState([]);
  const [loading, setLoading] = React.useState(configured);
  const [error, setError] = React.useState(null);

  // Values are fetched one at a time and held only while the pane is mounted.
  // Closing settings unmounts this, which is what re-hides every reveal.
  const [revealed, setRevealed] = React.useState({});

  const [editing, setEditing] = React.useState(null); // { name, value, label, isNew }
  const [formError, setFormError] = React.useState(null);
  const [saving, setSaving] = React.useState(false);
  const [removing, setRemoving] = React.useState(null);

  const reload = React.useCallback(async () => {
    if (!configured) {
      setEntries([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      setEntries(await client.secrets.list());
      setError(null);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setLoading(false);
    }
  }, [client, configured]);

  React.useEffect(() => {
    reload();
  }, [reload]);

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
      const value = await client.secrets.get(name);
      setRevealed((prev) => ({ ...prev, [name]: value ?? "" }));
    } catch (failure) {
      toast({
        title: "Could not read that secret",
        description: failure.message,
        variant: "danger",
      });
    }
  }

  async function save() {
    if (!editing) return;
    const name = editing.name.trim();
    if (!NAME.test(name)) {
      setFormError(
        "A name must start with a letter or underscore and hold only letters, digits and underscores.",
      );
      return;
    }
    if (editing.isNew && entries.some((e) => e.name === name)) {
      setFormError(
        `${name} already exists. Edit it instead, or choose another name.`,
      );
      return;
    }
    // A key the app looks up by name has a row of its own above, with a Test
    // button and no delete. Letting a second one in through this dialog would
    // put the same key on screen twice.
    if (editing.isNew && REQUIRED_NAMES.includes(name)) {
      setFormError(
        `${name} has a row of its own at the top of this pane. Set it there.`,
      );
      return;
    }
    if (editing.isNew && editing.value === "") {
      setFormError("A new secret needs a value.");
      return;
    }
    setSaving(true);
    try {
      // An edit that only renames the label should not blank the key, so an
      // empty field means "keep what is stored" rather than "store nothing".
      const value =
        !editing.isNew && editing.value === ""
          ? ((await client.secrets.get(name)) ?? "")
          : editing.value;
      await client.secrets.set(name, value, editing.label.trim());
      setEditing(null);
      setFormError(null);
      // a changed value makes an open reveal a lie
      setRevealed((prev) => {
        const next = { ...prev };
        delete next[name];
        return next;
      });
      await reload();
    } catch (failure) {
      setFormError(failure.message);
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!removing) return;
    try {
      await client.secrets.remove(removing.name);
      setRemoving(null);
      await reload();
    } catch (failure) {
      toast({
        title: "Could not remove that secret",
        description: failure.message,
        variant: "danger",
      });
    }
  }

  if (!configured) {
    return (
      <EmptyState
        icon={KeyRound}
        title="No workspace folder yet"
        description="Secrets live in the workspace folder. Choose one in Workspace settings and this pane fills in."
      />
    );
  }

  // The keys the app needs by name have their own section above, so they would
  // otherwise be listed twice - once with a Test button and once without.
  const rest = entries.filter((entry) => !REQUIRED_NAMES.includes(entry.name));

  return (
    <div className="w-full">
      <RequiredKeys entries={entries} loading={loading} onChanged={reload} />

      <SettingsSection flat
        title="Keys and tokens"
        description="Credentials your plugins and agents use. They are stored as plain text in the secrets folder of your workspace, not encrypted - anyone who can read that folder can read them. Keep the folder somewhere you would keep a password file."
      >
        <SettingsCard className="flex flex-col gap-0.5 p-1.5">
          {loading ? (
            <SkeletonRow lines={3} className="p-2" />
          ) : error ? (
            <p className="p-2 text-[0.6875rem] leading-relaxed text-muted-foreground">
              {error}
            </p>
          ) : rest.length ? (
            rest.map((entry) => (
              <SecretRow
                key={entry.name}
                entry={entry}
                revealed={revealed[entry.name] !== undefined}
                value={revealed[entry.name]}
                onToggle={() => toggleReveal(entry.name)}
                onCopy={() => client.secrets.get(entry.name)}
                onEdit={() => {
                  setFormError(null);
                  setEditing({
                    name: entry.name,
                    value: "",
                    label: entry.label ?? "",
                    isNew: false,
                  });
                }}
                onRemove={() => setRemoving(entry)}
              />
            ))
          ) : (
            <EmptyState
              className="py-8"
              icon={KeyRound}
              title="No secrets yet"
              description="Add an API key or token here once, then refer to it by name wherever a plugin asks for one."
            />
          )}
        </SettingsCard>

        <div className="flex">
          <Button
            variant="subtle"
            size="xs"
            onClick={() => {
              setFormError(null);
              setEditing({ name: "", value: "", label: "", isNew: true });
            }}
          >
            <Plus />
            Add secret
          </Button>
        </div>
      </SettingsSection>

      <SettingsSection flat title="Using a secret">
        <SettingsCard className="flex flex-col gap-2.5">
          <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
            Anywhere the app takes a config value - an MCP server header, an
            OpenAPI auth field, a plugin setting - write the reference instead
            of the key itself. Inertia swaps in the stored value at the moment
            it is used, so a config you export or share names the secret without
            carrying it.
          </p>
          <div className="flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-lg fill-control px-2.5 py-1.5 font-mono text-xs text-foreground">
              {EXAMPLE}
            </code>
            <CopyButton value={EXAMPLE} label="Copy example" />
          </div>
        </SettingsCard>
      </SettingsSection>

      <SecretDialog
        open={Boolean(editing)}
        onOpenChange={(open) => {
          if (!open) {
            setEditing(null);
            setFormError(null);
          }
        }}
        editing={editing}
        onChange={(patch) => setEditing((prev) => ({ ...prev, ...patch }))}
        onSave={save}
        saving={saving}
        error={formError}
        allowName={editing?.isNew}
        allowLabel
      />

      <ConfirmDialog
        open={Boolean(removing)}
        onOpenChange={(open) => !open && setRemoving(null)}
        destructive
        title={`Remove ${removing?.name ?? "this secret"}?`}
        description="The value is deleted from the workspace and cannot be recovered from here. Anything still referring to it by name will fail until you add it again."
        confirmLabel="Remove secret"
        onConfirm={remove}
      />
    </div>
  );
}
