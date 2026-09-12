import * as React from "react";
import { ExternalLink, Icon } from "@/components/icons";
import { cn } from "@/lib/utils";
import { tools as agentTools } from "@/lib/agent";
import { FilePreview, useLocalImage } from "@/features/chat/FilePreview";
import { FILE_KIND_ICON, baseName, fileKind, formatBytes } from "@/features/chat/thread-files";

/**
 * What the agent is handing over.
 *
 * Not a tool row. Everything else in a transcript is a record of something that
 * happened and folds away to one line; this is the thing the person asked for,
 * so it is drawn open, where it happened, as a small shelf of files to click.
 *
 * ── Every file, including the ones nothing here has heard of ───────────────
 * There is deliberately no "unsupported file type" state. A picture gets a
 * thumbnail and opens in the viewer, because that is what a picture is. Every
 * other file - a component, a PDF, a spreadsheet, a `.bin` - is a name, a size
 * and a way to open it, and opens in whatever the operating system uses for it.
 * That second path has no list of extensions in it and cannot go out of date,
 * which is the point: a viewer that knows six formats is a viewer that fails on
 * the seventh, and the failure lands on the one file the person wanted.
 */

/** A thumbnail when the bytes can be read, the file's glyph otherwise. */
function Face({ file, isImage }) {
  const disk = useLocalImage(file.path, isImage);

  if (isImage && disk.url) {
    return <img src={disk.url} alt="" className="size-9 shrink-0 rounded-lg object-cover" />;
  }
  return (
    <span className="grid size-9 shrink-0 place-items-center rounded-lg fill-whisper text-muted-foreground">
      <Icon name={FILE_KIND_ICON[fileKind(file.name)] ?? "File"} className="size-4" />
    </span>
  );
}

function Row({ file, onOpen }) {
  const isImage = fileKind(file.name) === "image";
  const size = formatBytes(file.bytes);

  return (
    <button
      type="button"
      title={file.path}
      onClick={() => (isImage ? onOpen(file) : agentTools.openPath(file.path))}
      className={cn(
        "group/row flex w-full items-center gap-2.5 rounded-xl px-2 py-1.5 text-left outline-none",
        "transition-colors duration-100 ease-out hover:fill-nav focus-visible:fill-nav"
      )}
    >
      <Face file={file} isImage={isImage} />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-[13px] text-foreground">{file.name}</span>
        {/* Right to left, so a deep path keeps the end that identifies it. */}
        <span dir="rtl" className="truncate text-left text-[11px] text-muted-foreground">
          <bdi>{file.path}</bdi>
        </span>
      </span>
      {size ? (
        <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/70">{size}</span>
      ) : null}
      {isImage ? null : (
        <ExternalLink
          aria-hidden="true"
          className="size-3.5 shrink-0 text-muted-foreground/0 transition-colors group-hover/row:text-muted-foreground/70"
        />
      )}
    </button>
  );
}

export function PresentedFiles({ files, note, className, ...props }) {
  const [open, setOpen] = React.useState(null);
  const list = React.useMemo(
    () =>
      (Array.isArray(files) ? files : [])
        .filter((one) => one?.path)
        .map((one) => ({ ...one, name: one.name || baseName(one.path) })),
    [files]
  );
  if (!list.length) return null;

  return (
    <div className={cn("my-2 rounded-2xl card-surface-subtle p-1.5", className)} {...props}>
      <p className="px-2 pb-1 pt-1.5 text-[11px] text-muted-foreground">
        {note || (list.length === 1 ? "Here is the file" : `Here are the ${list.length} files`)}
      </p>
      <div className="flex flex-col">
        {list.map((file) => (
          <Row key={file.path} file={file} onOpen={setOpen} />
        ))}
      </div>
      {/* The viewer fetches the bytes itself from the path, so nothing is read
          off disk until a picture is actually opened. It stays mounted with
          nothing in it so that closing can be watched rather than cut. */}
      <FilePreview
        file={open ? { name: open.name, kind: "image", path: open.path, size: open.bytes } : null}
        onClose={() => setOpen(null)}
      />
    </div>
  );
}
