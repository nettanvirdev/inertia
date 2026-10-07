import * as React from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogFooter, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { SettingsField } from "../../SettingsRow";

/**
 * Where a key gets typed.
 *
 * One dialog for both lists, because "paste a value" is the same act whether
 * the key is one the app needs by name or one the user invented. What differs
 * is only which fields exist: a key the backend looks up by a fixed name
 * cannot be renamed, and a label the catalogue already wrote is not the user's
 * to edit. Both are absences rather than disabled controls - a field you may
 * not use is worse than a field that is not there.
 */
export function SecretDialog({
  open,
  onOpenChange,
  editing,
  onChange,
  onSave,
  saving,
  error,
  /** The name field, for a key the user is inventing. */
  allowName = false,
  /** The description field, for a key the catalogue has no words for. */
  allowLabel = false,
  title,
  confirmLabel,
}) {
  if (!editing) {
    return <Dialog open={false} onOpenChange={onOpenChange} size="md" ariaLabel="Secret" />;
  }

  const heading = title ?? (editing.isNew ? "Add a secret" : `Edit ${editing.name}`);

  return (
    <Dialog open={open} onOpenChange={onOpenChange} size="md" ariaLabel={heading}>
      <DialogTitle>{heading}</DialogTitle>
      <DialogBody className="flex flex-col gap-3">
        {allowName ? (
          <SettingsField
            label="Name"
            htmlFor="secret-name"
            description="Letters, digits and underscores, starting with a letter or an underscore. GITHUB_TOKEN, for example."
          >
            <Input
              id="secret-name"
              size="sm"
              autoFocus
              spellCheck={false}
              autoComplete="off"
              placeholder="GITHUB_TOKEN"
              className="font-mono"
              value={editing.name}
              onChange={(e) => onChange({ name: e.target.value })}
            />
          </SettingsField>
        ) : null}

        <SettingsField
          label="Value"
          htmlFor="secret-value"
          description={
            editing.isNew
              ? "Pasted as-is. Leading and trailing spaces are kept, so trim them yourself."
              : "The stored value is not shown here. Type a new one to replace it, or leave this blank to keep it."
          }
        >
          <Input
            id="secret-value"
            size="sm"
            type="password"
            autoFocus={!allowName}
            spellCheck={false}
            autoComplete="off"
            placeholder="Paste the key or token"
            className="font-mono"
            value={editing.value ?? ""}
            onChange={(e) => onChange({ value: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !saving) onSave();
            }}
          />
        </SettingsField>

        {allowLabel ? (
          <SettingsField
            label="Label"
            htmlFor="secret-label"
            description="Optional. What this key is for, so a list of names stays readable."
          >
            <Input
              id="secret-label"
              size="sm"
              placeholder="Personal access token for the docs repo"
              value={editing.label ?? ""}
              onChange={(e) => onChange({ label: e.target.value })}
            />
          </SettingsField>
        ) : null}

        {error ? (
          <p className="animate-fade-in text-[0.6875rem] leading-relaxed text-destructive-ink">
            {error}
          </p>
        ) : null}
      </DialogBody>
      <DialogFooter>
        <Button variant="secondary" size="pill" onClick={() => onOpenChange(false)}>
          Cancel
        </Button>
        <Button variant="primary" size="pill" disabled={saving} onClick={onSave}>
          {confirmLabel ?? (editing.isNew ? "Add secret" : "Save")}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
