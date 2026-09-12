import * as React from "react";
import { ChevronRight, FilePen, RotateCcw } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { Dialog, DialogBody, DialogFooter, DialogTitle } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { ToolDiff } from "@/features/chat/ToolDiff";
import { isSnapshotAvailable, loadDiff, revertTurn } from "@/lib/snapshot";

/**
 * What a reply changed on disk, and the way back.
 *
 * Drawn once under the reply, after the prose and the tool cards, because it
 * is a fact about the whole turn: the tool cards say what the agent asked
 * for, this says what the folder looks like as a result, which is not the
 * same thing once a command has run. Each file opens its diff; Revert puts
 * the folder back to the snapshot taken before the turn's first write, and
 * says so in place afterwards rather than removing the strip - the record of
 * what was changed is still true, it is just no longer the state of the disk.
 */

const STATUS_LABEL = { A: "added", M: "changed", D: "deleted", R: "renamed", C: "copied", T: "changed" };

function Counts({ file }) {
  if (file.additions == null && file.deletions == null) return <span className="text-muted-foreground">binary</span>;
  return (
    <span className="tabular-nums">
      {file.additions ? <span className="text-success-ink">+{file.additions}</span> : null}
      {file.additions && file.deletions ? " " : null}
      {file.deletions ? <span className="text-destructive-ink">-{file.deletions}</span> : null}
    </span>
  );
}

export function ChangesStrip({ changes }) {
  const { toast } = useToast();
  const [openFile, setOpenFile] = React.useState(null);
  const [diff, setDiff] = React.useState("");
  const [confirming, setConfirming] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [reverted, setReverted] = React.useState(false);
  /**
   * Folded, because the summary is the point.
   *
   * A turn that scaffolds a project changes thirty files, and thirty file names
   * under the reply is longer than the reply. What the line has to say is that
   * something was written and how much - "32 files changed, +6594 -89" - and
   * which files is the follow-up question, not the headline.
   */
  const [open, setOpen] = React.useState(false);
  const files = changes?.files ?? [];
  const canRevert = isSnapshotAvailable() && Boolean(changes?.from) && !reverted;

  // Keyed on the three strings the diff is actually addressed by, not on the
  // `changes` object: a parent re-render that hands us an equal-but-new object
  // would otherwise throw the loaded diff away and fetch it again, and the
  // panel would flicker back through its loading state for no reason.
  const { cwd, from, to } = changes ?? {};
  React.useEffect(() => {
    if (!openFile) return undefined;
    let alive = true;
    setDiff("");
    loadDiff({ cwd, from, to, file: openFile })
      .then((text) => alive && setDiff(text || "(no textual diff)"))
      .catch((error) => alive && setDiff(error?.message ?? "Could not load the diff."));
    return () => {
      alive = false;
    };
  }, [openFile, cwd, from, to]);

  if (!files.length) return null;

  const additions = files.reduce((sum, f) => sum + (f.additions ?? 0), 0);
  const deletions = files.reduce((sum, f) => sum + (f.deletions ?? 0), 0);

  const revert = async () => {
    setBusy(true);
    try {
      const result = await revertTurn({ cwd: changes.cwd, to: changes.from, since: changes.to });
      if (result?.reverted) {
        setReverted(true);
        toast({ title: "Reverted", description: `${files.length} file${files.length === 1 ? "" : "s"} put back the way they were before this reply.` });
      } else {
        toast({ title: "Could not revert", description: result?.reason ?? "Nothing was changed.", variant: "danger" });
      }
    } catch (error) {
      toast({ title: "Could not revert", description: error?.message ?? "Nothing was changed.", variant: "danger" });
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  };

  return (
    <div
      className={cn(
        "card-surface-subtle my-2 rounded-xl px-3 py-2 text-[12px] transition-opacity duration-[var(--motion-base)] ease-[var(--ease-out)]",
        reverted && "opacity-70"
      )}
    >
      <div className="flex items-center gap-2">
        {/* The whole summary line is the control. Revert sits outside it: a
            button that reverts thirty files must not be something anyone can
            hit while reaching for a disclosure triangle. */}
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
          className="-mx-1 flex min-w-0 flex-1 items-center gap-2 rounded-lg px-1 py-0.5 text-left outline-none transition-colors duration-100 hover:fill-nav focus-visible:fill-nav"
        >
          <FilePen className="text-muted-foreground size-3.5 shrink-0" aria-hidden="true" />
          <span className="font-medium">
            {files.length} file{files.length === 1 ? "" : "s"} {reverted ? "were changed, then reverted" : "changed"}
          </span>
          <span className="text-muted-foreground tabular-nums">
            {additions ? <span className="text-success-ink">+{additions}</span> : null}
            {additions && deletions ? " " : null}
            {deletions ? <span className="text-destructive-ink">-{deletions}</span> : null}
          </span>
          <ChevronRight
            aria-hidden="true"
            className={cn(
              "text-muted-foreground size-3.5 shrink-0 transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
              open && "rotate-90"
            )}
          />
        </button>
        {canRevert ? (
          <Button size="xs" variant="ghost" className="ml-auto shrink-0" onClick={() => setConfirming(true)} disabled={busy}>
            <RotateCcw className="size-3.5" aria-hidden="true" />
            Revert
          </Button>
        ) : null}
      </div>
      <Collapse open={open}>
        <ul
          // No `overscroll-contain`: a list inside the transcript must hand the
          // wheel back to the transcript when it reaches its end.
          className="no-scrollbar mt-1.5 max-h-[var(--tool-max-height,320px)] space-y-0.5 overflow-y-auto"
        >
          {files.map((file) => (
            <li key={file.path} className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => setOpenFile(file.path)}
                className="text-foreground-secondary hover:text-foreground min-w-0 truncate text-left font-mono text-[12px] underline-offset-2 hover:underline"
                title={`Show what changed in ${file.path}`}
              >
                {file.from ? `${file.from} → ` : null}
                {file.path}
              </button>
              <span className="text-muted-foreground shrink-0">{STATUS_LABEL[file.status] ?? file.status}</span>
              <Counts file={file} />
            </li>
          ))}
        </ul>
      </Collapse>

      {/* One size, whatever is inside it.
          The diff arrives a moment after the panel does, and a panel sized to
          its content would open at the height of "Loading the diff…" and then
          snap to the height of nine hundred lines under the reader's cursor.
          So the body is a fixed viewport that scrolls: the panel that opens is
          the panel that stays, and a long file scrolls inside it rather than
          running off the screen. `pr-8` keeps a long path clear of the close
          button, and `break-all` keeps it on one line's worth of decisions. */}
      <Dialog open={Boolean(openFile)} onOpenChange={(open) => !open && setOpenFile(null)} size="lg" ariaLabel="File diff">
        <DialogTitle className="pr-8 font-mono text-[13px] break-all">{openFile}</DialogTitle>
        <DialogBody className="no-scrollbar h-[min(60vh,32rem)] overflow-y-auto">
          {diff ? <ToolDiff diff={diff} /> : <p className="text-muted-foreground text-xs">Loading the diff…</p>}
        </DialogBody>
      </Dialog>

      <Dialog open={confirming} onOpenChange={setConfirming} size="sm" ariaLabel="Revert this reply's changes">
        <DialogTitle>Put these files back?</DialogTitle>
        <DialogBody>
          <p className="text-foreground-secondary text-sm">
            {files.length} file{files.length === 1 ? "" : "s"} will go back to the way {files.length === 1 ? "it" : "they"} were before this
            reply. Files the reply created are removed. Anything changed since, by you or a later reply, is lost too.
          </p>
        </DialogBody>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setConfirming(false)} disabled={busy}>
            Keep them
          </Button>
          <Button onClick={revert} disabled={busy}>
            {busy ? "Reverting…" : "Revert"}
          </Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
