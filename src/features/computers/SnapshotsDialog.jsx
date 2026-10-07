import * as React from "react";
import { Camera, RotateCcw } from "@/components/icons";
import { relativeTime } from "@/data";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { computers as machines } from "@/lib/computers";

/**
 * The snapshots a machine actually has.
 *
 * There used to be a counter on the detail page that nothing incremented, no
 * list and no way back. All three parts exist now: Docker commits the container
 * to an image, Daytona takes a backup, local copies the folder, and all three
 * answer the same list.
 *
 * Restoring is deliberately behind a confirmation with the consequence spelled
 * out, because on both real providers a restore does not roll the machine back
 * in place - it replaces it. Anything written since the snapshot is gone, and
 * that is not something to discover afterwards.
 */
export function SnapshotsDialog({ open, onOpenChange, computer }) {
  const { toast } = useToast();
  const [rows, setRows] = React.useState(null);
  const [error, setError] = React.useState(null);
  const [busy, setBusy] = React.useState(false);
  const [restoring, setRestoring] = React.useState(null);

  const load = React.useCallback(async () => {
    setError(null);
    try {
      setRows(await machines.snapshots(computer.id));
    } catch (problem) {
      setError(problem?.message ?? String(problem));
      setRows([]);
    }
  }, [computer.id]);

  React.useEffect(() => {
    if (open) load();
  }, [open, load]);

  async function take() {
    setBusy(true);
    try {
      const snap = await machines.snapshot(computer.id);
      toast({ variant: "success", title: "Snapshot taken", description: snap.name });
      await load();
    } catch (problem) {
      toast({ variant: "error", title: "Snapshot failed", description: problem?.message });
    } finally {
      setBusy(false);
    }
  }

  async function restore(snapshot) {
    setBusy(true);
    try {
      await machines.restore(computer.id, snapshot.id);
      toast({
        variant: "success",
        title: `${computer.name} restored`,
        description: `Back to ${snapshot.name}.`,
      });
      onOpenChange(false);
    } catch (problem) {
      toast({ variant: "error", title: "Restore failed", description: problem?.message });
    } finally {
      setBusy(false);
      setRestoring(null);
    }
  }

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange} size="md">
        <DialogTitle>Snapshots</DialogTitle>
        <DialogDescription>A copy of {computer.name} as it was, to come back to.</DialogDescription>

        <DialogBody>
          <ScrollArea className="max-h-[min(24rem,calc(100dvh-20rem))] -mx-1 px-1">
            {rows === null ? (
              <div className="flex justify-center py-8">
                <Spinner />
              </div>
            ) : error ? (
              <p className="py-6 text-center text-[13px] text-muted-foreground">{error}</p>
            ) : rows.length === 0 ? (
              <p className="py-6 text-center text-[13px] text-muted-foreground">
                No snapshots yet.
              </p>
            ) : (
              <div className="flex flex-col gap-1">
                {rows.map((snapshot) => (
                  <div
                    key={snapshot.id}
                    className="flex items-center gap-3 rounded-xl fill-whisper px-3 py-2"
                  >
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-[13px] text-foreground">{snapshot.name}</p>
                      <p className="text-[11px] text-muted-foreground">
                        {snapshot.at ? relativeTime(snapshot.at) : "no date"}
                        {snapshot.size ? ` · ${snapshot.size}` : ""}
                      </p>
                    </div>
                    <Button
                      size="xs"
                      variant="subtle"
                      disabled={busy}
                      onClick={() => setRestoring(snapshot)}
                    >
                      <RotateCcw />
                      Restore
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </ScrollArea>
        </DialogBody>

        <DialogFooter className="sm:justify-between">
          <Button
            size="pill"
            variant="subtle"
            disabled={busy || computer.status !== "running"}
            onClick={take}
          >
            <Camera />
            {busy ? "Working…" : "Take one now"}
          </Button>
          <Button size="pill" variant="secondary" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </DialogFooter>
      </Dialog>

      <ConfirmDialog
        open={Boolean(restoring)}
        onOpenChange={(next) => !next && setRestoring(null)}
        destructive
        title={`Restore ${computer.name}?`}
        description={
          `The machine is replaced with the one from this snapshot. Everything written ` +
          `since ${restoring?.at ? relativeTime(restoring.at) : "it was taken"} is lost, and ` +
          `anything running on it stops.`
        }
        confirmLabel="Restore"
        onConfirm={() => restoring && restore(restoring)}
      />
    </>
  );
}
