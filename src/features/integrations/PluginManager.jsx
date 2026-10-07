import * as React from "react";
import { CircleAlert, Plus, SearchX } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useCollection, useWorkspace } from "@/lib/workspace";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { ScrollArea } from "@/components/ui/scroll-area";
import { EmptyState } from "@/components/ui/empty-state";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import {
  Dialog,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogFooter,
} from "@/components/ui/dialog";
import { GUTTER } from "@/components/layout/View";
import { PluginRow } from "./PluginRow";

/**
 * The list/add/edit/remove machinery, written once.
 *
 * Every tab in this view is the same screen over a different folder, so the
 * only thing a plugin kind supplies is a descriptor: how to draw a row, what
 * an empty record looks like, and the form. A fifth kind is a new descriptor
 * and nothing else.
 *
 * Writes go through `useCollection`, which means they hit the disk and throw on
 * failure - so the dialog stays open and shows the reason rather than closing
 * on a save that did not happen.
 */
export function PluginManager({ kind, query, layout, addToken }) {
  const { client, configured } = useWorkspace();
  const { toast } = useToast();
  const { items, loading, error, save, patch, remove } = useCollection(kind.collection, {
    enabled: configured,
  });

  const [draft, setDraft] = React.useState(null);
  const [editingId, setEditingId] = React.useState(null);
  const [saving, setSaving] = React.useState(false);
  const [formError, setFormError] = React.useState(null);
  const [pendingRemove, setPendingRemove] = React.useState(null);

  const openAdd = React.useCallback(
    (seed) => {
      setEditingId(null);
      setFormError(null);
      setDraft({ ...kind.blank(), ...(seed ?? {}) });
    },
    [kind]
  );

  // The primary button lives in the view header, above this component. The
  // token is how a press reaches down here; the ref keeps a stale token from
  // popping the dialog open when a tab is remounted.
  const seenToken = React.useRef(addToken);
  React.useEffect(() => {
    if (addToken === seenToken.current) return;
    seenToken.current = addToken;
    openAdd();
  }, [addToken, openAdd]);

  function openEdit(record) {
    setEditingId(record.id);
    setFormError(null);
    setDraft(kind.toDraft ? kind.toDraft(record) : { ...record });
  }

  function closeDialog() {
    setDraft(null);
    setEditingId(null);
    setFormError(null);
  }

  const filtered = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter((item) => kind.search(item).toLowerCase().includes(q));
  }, [items, query, kind]);

  async function commit() {
    const problem = kind.validate?.(draft);
    if (problem) {
      setFormError(problem);
      return;
    }
    setSaving(true);
    setFormError(null);
    try {
      const record = kind.toRecord(draft, { editingId });
      const saved = await save(editingId ? { ...record, id: editingId } : record);
      const note = await kind.afterSave?.(saved, draft, { client, patch });
      toast({
        variant: "success",
        title: `${saved.name || kind.noun} ${editingId ? "updated" : "added"}`,
        description: note ?? kind.savedNote?.(saved) ?? undefined,
      });
      closeDialog();
    } catch (failure) {
      setFormError(failure.message);
    } finally {
      setSaving(false);
    }
  }

  async function toggle(record) {
    try {
      await patch(record.id, { enabled: !record.enabled });
    } catch (failure) {
      toast({ variant: "danger", title: "Could not save that", description: failure.message });
    }
  }

  async function confirmRemove() {
    const doomed = pendingRemove;
    if (!doomed) return;
    try {
      await remove(doomed.id);
      toast({ variant: "warning", title: `${doomed.name || doomed.id} removed` });
    } catch (failure) {
      toast({ variant: "danger", title: "Could not remove that", description: failure.message });
    } finally {
      setPendingRemove(null);
    }
  }

  const Form = kind.Form;
  const Catalog = kind.Catalog;

  return (
    <>
      <ScrollArea className={cn("min-h-0 flex-1 pt-2 pb-8", GUTTER)}>
        {/* Mounted fresh on every tab switch (the view keys on the kind), so
            the arrival is the tab change. */}
        <div className="flex w-full animate-fade-in flex-col gap-8">
          <section className="flex flex-col gap-2">
            <h2 className="flex items-center gap-2 text-[11px] font-semibold text-muted-foreground">
              {kind.listTitle}
              <span className="font-normal tabular-nums">{filtered.length}</span>
            </h2>

            {error ? (
              <EmptyState
                icon={CircleAlert}
                title="That folder could not be read"
                description={error}
              />
            ) : loading ? (
              <div className="flex items-center gap-2 py-6 text-[13px] text-muted-foreground">
                <Spinner size="sm" />
                Reading the workspace folder
              </div>
            ) : filtered.length ? (
              <div
                className={cn(
                  layout === "grid"
                    ? "grid gap-2 md:grid-cols-2 2xl:grid-cols-3"
                    : "flex flex-col gap-1.5"
                )}
              >
                {filtered.map((record) => {
                  const row = kind.row(record);
                  return (
                    <PluginRow
                      key={record.id}
                      glyph={row.glyph}
                      title={row.title}
                      subtitle={row.subtitle}
                      badges={row.badges}
                      enabled={record.enabled !== false}
                      onToggle={() => toggle(record)}
                      onEdit={() => openEdit(record)}
                      onRemove={() => setPendingRemove(record)}
                    />
                  );
                })}
              </div>
            ) : items.length ? (
              <EmptyState
                icon={SearchX}
                title="Nothing matches"
                description={`None of your ${items.length} ${kind.nounPlural} match "${query.trim()}".`}
              />
            ) : (
              <EmptyState
                icon={kind.icon}
                title={kind.empty.title}
                description={kind.empty.description}
                action={
                  <Button variant="primary" size="sm" onClick={() => openAdd()}>
                    <Plus />
                    {kind.addLabel}
                  </Button>
                }
              />
            )}
          </section>

          {Catalog ? <Catalog installed={items} onAdd={openAdd} /> : null}
        </div>
      </ScrollArea>

      <Dialog
        open={Boolean(draft)}
        onOpenChange={(v) => !v && closeDialog()}
        size={kind.dialogSize ?? "lg"}
      >
        <DialogTitle>{editingId ? `Edit ${kind.noun}` : kind.addLabel}</DialogTitle>
        <DialogDescription>{kind.dialogHint}</DialogDescription>
        <DialogBody className="mt-4">
          <ScrollArea className="max-h-[min(30rem,60dvh)] pr-1" fade>
            {draft ? (
              <Form
                value={draft}
                onChange={(next) => setDraft((current) => ({ ...current, ...next }))}
                editing={Boolean(editingId)}
              />
            ) : null}
          </ScrollArea>
          {formError ? (
            <p className="mt-3 animate-fade-in text-[11px] text-destructive-ink">{formError}</p>
          ) : null}
        </DialogBody>
        <DialogFooter>
          <Button variant="secondary" size="sm" onClick={closeDialog} disabled={saving}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" onClick={commit} disabled={saving}>
            {saving ? <Spinner size="sm" /> : null}
            {editingId ? "Save changes" : kind.addLabel}
          </Button>
        </DialogFooter>
      </Dialog>

      <ConfirmDialog
        open={Boolean(pendingRemove)}
        onOpenChange={(v) => !v && setPendingRemove(null)}
        destructive
        title={pendingRemove ? `Remove ${pendingRemove.name || pendingRemove.id}?` : ""}
        description={pendingRemove ? kind.removeHint(pendingRemove) : ""}
        confirmLabel="Remove"
        onConfirm={confirmRemove}
      />
    </>
  );
}
