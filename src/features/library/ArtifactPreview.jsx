import * as React from "react";
import { Icon } from "@/components/icons";
import { cn } from "@/lib/utils";
import { FILE_KIND_ICON, formatBytes } from "@/features/chat/thread-files";
import { mimeFor, openTargetFor } from "@/features/library/artifacts";
import { useWorkspace } from "@/lib/workspace";

/**
 * What an artifact actually looks like.
 *
 * Every preview here is the real thing or an admission that there is nothing to
 * show. The version this replaces drew a frame at the right aspect ratio with
 * the filename in it, a fake waveform seeded from the id and a "first line" it
 * synthesised from the file extension - all of which read, at a glance, as a
 * thumbnail of a file the app had opened. It had not opened anything.
 *
 * So: an image with bytes is an `<img>`, a recording is an `<audio>` or a
 * `<video>`, text with bytes is the text, and a path is a path. The last case
 * is the common one for anything an agent wrote on a computer, and the honest
 * thing to put in that space is where it lives and how to open it.
 */

/** A picture, in the frame, with nothing between it and the viewer. */
function ImagePreview({ artifact, className }) {
  return (
    <div
      className={cn(
        "grid place-items-center overflow-hidden rounded-2xl bg-card-darker p-2",
        className
      )}
    >
      <img
        src={artifact.dataUrl}
        alt={artifact.name}
        className="max-h-full max-w-full rounded-xl object-contain"
      />
    </div>
  );
}

/**
 * A recording, played where it is.
 *
 * Same bytes and the same route as the image: a data URL goes straight into the
 * element and the browser does the rest. A recording used to fall through to
 * the "nothing here can show it" card, and the only offer was to save a copy
 * and open it somewhere else - which Chromium, the thing drawing this window,
 * can do perfectly well itself.
 *
 * The bytes are not always to hand. Nothing the composer accepts is a
 * recording, so in practice a sound or a film arrives as a path an agent wrote,
 * and the ones inside the Inertia folder can be read back through the same
 * bridge the avatar uses. A path on some other disk cannot: `readBytes`
 * refuses anything outside the workspace, on purpose, and that case falls back
 * to the location and the button that opens it.
 *
 * Nothing plays on its own. A file list is not a place anyone expects noise to
 * start, so both elements load their metadata and wait to be asked.
 */
function MediaPreview({ artifact, className, video, source }) {
  const Element = video ? "video" : "audio";
  return (
    <div
      className={cn(
        "grid place-items-center overflow-hidden rounded-2xl bg-card-darker p-3",
        className
      )}
    >
      <Element
        src={source}
        controls
        preload="metadata"
        aria-label={artifact.name}
        className={cn("max-h-full", video ? "max-w-full rounded-xl" : "w-full")}
      />
    </div>
  );
}

/**
 * The bytes of a workspace file, as something an element can play.
 *
 * Only for the artifact that is open. A Library page is forty cards and reading
 * forty media files into forty data URLs to draw forty players nobody has asked
 * to hear would cost more memory than the whole rest of the screen - so the
 * grid gets the card that says what the file is, and the dialog gets the
 * player.
 */
function useWorkspaceMedia(artifact, mime, enabled) {
  const { client, native } = useWorkspace();
  const [source, setSource] = React.useState(null);
  const path = artifact?.path;

  React.useEffect(() => {
    setSource(null);
    if (!enabled || !native || !path) return undefined;
    // Outside the folder. `readBytes` would refuse it, and asking anyway would
    // be a rejected promise per artifact for a case that is entirely knowable
    // from the path.
    if (openTargetFor(path).how !== "workspace") return undefined;

    let alive = true;
    client
      .readBytes(path)
      .then((base64) => {
        if (alive && base64) setSource(`data:${mime};base64,${base64}`);
      })
      .catch(() => {
        /* a file that will not read is a file with no player, which is the
           same thing the path card already says */
      });
    return () => {
      alive = false;
    };
  }, [client, native, path, mime, enabled]);

  return source;
}

/**
 * The file, as far down as the box goes.
 *
 * Scrolls rather than truncates: a 200KB attachment is a readable file and the
 * only reason to cut it off would be to save a scrollbar.
 */
function TextPreview({ artifact, className, compact }) {
  // A card in a grid of forty gets the head of the file and no scrollbar; the
  // dialog gets all of it. Slicing rather than clipping with CSS keeps a 200KB
  // attachment out of the DOM forty times over.
  const body = compact ? String(artifact.text).split("\n").slice(0, 10).join("\n") : artifact.text;

  return (
    <div
      className={cn(
        "rounded-2xl bg-card-darker p-3.5",
        compact ? "overflow-hidden" : "overflow-auto",
        className
      )}
    >
      <pre className="font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-foreground/85">
        {body}
      </pre>
    </div>
  );
}

/** No blob, only a location - which is the whole of what is known about it. */
function PathPreview({ artifact, className }) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-2 rounded-2xl bg-card-darker px-5 py-6 text-center",
        className
      )}
    >
      <Icon
        name={FILE_KIND_ICON[artifact.kind] ?? "File"}
        className="size-5 text-muted-foreground/70"
        aria-hidden="true"
      />
      <p
        className="max-w-full truncate font-mono text-[11px] text-foreground/80"
        title={artifact.path}
      >
        {artifact.path}
      </p>
      <p className="text-[10.5px] leading-relaxed text-muted-foreground">
        This file lives on the computer, not in the conversation. Open it to see what is in it.
      </p>
    </div>
  );
}

/** Bytes are here, but not bytes a page can render - so say what they are. */
function OpaquePreview({ artifact, className }) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-2 rounded-2xl bg-card-darker px-5 py-6 text-center",
        className
      )}
    >
      <Icon
        name={FILE_KIND_ICON[artifact.kind] ?? "File"}
        className="size-5 text-muted-foreground/70"
        aria-hidden="true"
      />
      <p className="text-[11px] text-foreground/80">{artifact.name}</p>
      <p className="text-[10.5px] text-muted-foreground">
        {formatBytes(artifact.sizeBytes) ?? "Unknown size"} · nothing here can show it. Save a copy
        and open it where it belongs.
      </p>
    </div>
  );
}

/**
 * The media branch, as its own component so it may hold state.
 *
 * The hook that reads the file cannot live in `ArtifactPreview`: that function
 * returns early for every other kind, and a hook behind a return is a hook that
 * runs on some renders and not others.
 */
function MediaBranch({ artifact, className, mime, compact }) {
  const loaded = useWorkspaceMedia(artifact, mime, !compact && !artifact.dataUrl);
  const src = artifact.dataUrl ?? loaded;

  if (src) {
    return (
      <MediaPreview
        artifact={artifact}
        className={className}
        video={mime.startsWith("video/")}
        source={src}
      />
    );
  }
  // Still reading, or nothing to read. Either way the file has a location and
  // that is what there is to show; the alternative is an empty player that
  // never plays anything. `failed` is not rendered differently on purpose -
  // "this lives on the computer, open it there" is the true thing to say in
  // both cases, and a flash of a different card while a file loads is noise.
  return <PathPreview artifact={artifact} className={className} />;
}

export function ArtifactPreview({ artifact, className, compact = false }) {
  if (!artifact) return null;
  if (artifact.dataUrl && artifact.kind === "image") {
    return <ImagePreview artifact={artifact} className={className} />;
  }
  // Asked of the bytes and the name rather than the row's `kind`, which says
  // only "file" for a recording - `kind` decides which glyph a row gets, and
  // that is a coarser question than what a player needs to know.
  const mime = mimeFor(artifact);
  if (mime.startsWith("audio/") || mime.startsWith("video/")) {
    return <MediaBranch artifact={artifact} className={className} mime={mime} compact={compact} />;
  }
  if (artifact.text != null) {
    return <TextPreview artifact={artifact} className={className} compact={compact} />;
  }
  if (artifact.dataUrl) return <OpaquePreview artifact={artifact} className={className} />;
  return <PathPreview artifact={artifact} className={className} />;
}
