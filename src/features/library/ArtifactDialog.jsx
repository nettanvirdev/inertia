import * as React from "react";
import { Copy, Download, ExternalLink } from "@/components/icons";
import { tools as agentTools } from "@/lib/agent";
import { useWorkspace } from "@/lib/workspace";
import { copyText as writeClipboard } from "@/lib/clipboard";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogFooter,
} from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { ArtifactPreview } from "@/features/library/ArtifactPreview";
import {
  dataUrlPayload,
  hasContent,
  openTargetFor,
  saveTargetPath,
} from "@/features/library/artifacts";

/**
 * One artifact, opened.
 *
 * Shared by the Library and the chat's Files panel because the two lists hold
 * the same rows and used to disagree about what a click meant - the panel
 * raised "opening attachments is not available in this preview build" and the
 * Library raised "download queued" for a download that was never queued.
 *
 * The two kinds part company here, and the split is the one
 * `features/chat/thread-files.js` describes. An artifact that carries bytes can
 * be shown and written somewhere; an artifact that is only a path can be handed
 * to the operating system and to nothing else. Neither branch pretends to the
 * other's abilities.
 */

/**
 * Opening and saving, without a dialog attached.
 *
 * A hook rather than functions because both halves need the workspace client,
 * and both need to say what went wrong where the user is looking rather than
 * throwing into a console nobody has open.
 */
export function useArtifactActions() {
  const { client, native } = useWorkspace();
  const { toast } = useToast();

  /**
   * Hand a path to the OS.
   *
   * Two channels, chosen by `openTargetFor`: the workspace one refuses
   * anything outside the Inertia folder, which is most of what an agent
   * touches, so an absolute path goes to the unrestricted one instead.
   */
  const openPath = React.useCallback(
    async (path) => {
      const { how, target } = openTargetFor(path);
      if (how === "unknown") {
        toast({ title: "No path to open", variant: "warning" });
        return;
      }
      try {
        if (how === "os") await agentTools.openPath(target);
        else await client.reveal(target);
      } catch (error) {
        toast({
          title: "Could not open it",
          description:
            error?.message ??
            `${target} could not be opened. It may have been moved, or it may be on a computer this machine cannot reach.`,
          variant: "warning",
        });
      }
    },
    [client, toast]
  );

  /**
   * Write a copy of the bytes somewhere the user can find them.
   *
   * In the app that means the workspace's own `files/` directory, which exists
   * for exactly this ("attachments and anything an agent produced") and needs
   * no new IPC. In a browser preview there is no folder at all, so it falls
   * back to the browser's own download, which is the only honest option there.
   */
  const saveCopy = React.useCallback(
    async (artifact) => {
      if (!hasContent(artifact)) {
        toast({
          title: "Nothing to save",
          description: `${artifact?.name ?? "This file"} is a location on a computer, not a file this app is holding.`,
          variant: "warning",
        });
        return null;
      }

      if (!native) {
        downloadInBrowser(artifact);
        return null;
      }

      const relPath = saveTargetPath(artifact);
      try {
        const payload = dataUrlPayload(artifact.dataUrl);
        if (payload != null) await client.writeBytes(relPath, payload);
        else await client.writeFile(relPath, artifact.text ?? "");
        toast({
          title: "Saved to your Inertia folder",
          description: relPath,
          variant: "success",
          action: { label: "Show", onClick: () => client.reveal(relPath).catch(() => {}) },
        });
        return relPath;
      } catch (error) {
        toast({
          title: "Could not save it",
          description: error?.message ?? `${relPath} could not be written.`,
          variant: "warning",
        });
        return null;
      }
    },
    [client, native, toast]
  );

  const copyText = React.useCallback(
    async (value, what) => {
      if (await writeClipboard(value)) {
        toast({ title: "Copied", description: value, variant: "success" });
      } else {
        toast({
          title: `Could not copy the ${what}`,
          description: "This window has no access to the clipboard.",
          variant: "warning",
        });
      }
    },
    [toast]
  );

  return { openPath, saveCopy, copyText };
}

/**
 * The browser-preview fallback.
 *
 * A data URL and a Blob both work as an `<a download>` href here - there is no
 * sandbox in the way - and the anchor is removed immediately, so nothing is
 * left in the document after the click.
 */
function downloadInBrowser(artifact) {
  const href =
    artifact.dataUrl ??
    URL.createObjectURL(new Blob([artifact.text ?? ""], { type: "text/plain" }));
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.download = artifact.name || "artifact";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  if (!artifact.dataUrl) URL.revokeObjectURL(href);
}

/**
 * @param {object}   props
 * @param {object?}  props.artifact  a row from `collectThreadFiles`, or a
 *                                   Library item, which is a superset of one
 * @param {Array?}   props.facts     `[label, value]` pairs to list under the
 *                                   preview; the two screens know different
 *                                   things about the same file
 * @param {React.ReactNode?} props.trailing  extra footer buttons, after the
 *                                    shared ones, where a primary action goes
 */
export function ArtifactDialog({ artifact, facts = [], trailing = null, onOpenChange }) {
  const { openPath, saveCopy, copyText } = useArtifactActions();
  const carriesBytes = hasContent(artifact);

  return (
    <Dialog open={Boolean(artifact)} onOpenChange={onOpenChange} size="lg">
      {artifact ? (
        <>
          <DialogTitle>{artifact.name}</DialogTitle>
          <DialogDescription>
            {artifact.summary ??
              (carriesBytes ? "Attached in this conversation." : `Written to ${artifact.path}.`)}
          </DialogDescription>
          <DialogBody>
            <ArtifactPreview artifact={artifact} className="max-h-[22rem] min-h-[9rem]" />

            {facts.length ? (
              <dl className="mt-4 flex flex-col gap-2 rounded-xl fill-whisper p-4 text-[13px]">
                {facts.filter(Boolean).map(([label, value]) => (
                  <div key={label} className="flex items-baseline gap-3">
                    <dt className="w-24 shrink-0 text-[11px] text-muted-foreground">{label}</dt>
                    <dd className="min-w-0 flex-1 break-words text-foreground/90">{value}</dd>
                  </div>
                ))}
              </dl>
            ) : null}
          </DialogBody>

          <DialogFooter>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => copyText(artifact.path ?? artifact.name, "name")}
            >
              <Copy />
              {carriesBytes ? "Copy name" : "Copy path"}
            </Button>
            {carriesBytes ? (
              <Button variant="secondary" size="sm" onClick={() => saveCopy(artifact)}>
                <Download />
                Save a copy
              </Button>
            ) : (
              <Button variant="secondary" size="sm" onClick={() => openPath(artifact.path)}>
                <ExternalLink />
                Open on the computer
              </Button>
            )}
            {trailing}
          </DialogFooter>
        </>
      ) : null}
    </Dialog>
  );
}
