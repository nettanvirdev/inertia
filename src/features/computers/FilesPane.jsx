import * as React from "react";
import {
  ChevronRight,
  Download,
  Eye,
  File,
  FileText,
  Folder,
  FolderOpen,
  Image as ImageIcon,
  RotateCw,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { formatBytes, relativeTime } from "@/data";
import { ScrollArea } from "@/components/ui/scroll-area";
import { IconButton } from "@/components/ui/icon-button";
import { Button } from "@/components/ui/button";
import { ContextMenu } from "@/components/ui/context-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { Spinner } from "@/components/ui/spinner";
import { Dialog, DialogClose } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { computers as machines, joinPath, parentPath, workdirOf } from "@/lib/computers";

/** The image types worth showing rather than describing, and their MIME. */
const IMAGE_MIME = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  bmp: "image/bmp",
  ico: "image/x-icon",
  avif: "image/avif",
};

/** The MIME for a name, if it is an image we can draw. */
function imageMime(name) {
  const ext = String(name ?? "")
    .split(".")
    .pop()
    ?.toLowerCase();
  return (ext && IMAGE_MIME[ext]) || null;
}

/** Bigger than this and an image is offered as a download rather than drawn
 *  inline - a 40MB PNG as a data URL is a frozen window. */
const IMAGE_PREVIEW_LIMIT = 8_000_000;

/**
 * The machine's filesystem, as it actually is.
 *
 * This pane used to render a hand-written tree of a project nobody had - four
 * hundred lines of invented filenames with invented byte counts, and a preview
 * that said, in the file it was pretending to show, that it could not show
 * files. The provider has `listDir` and `readFile`, so now it uses them.
 *
 * Loaded a directory at a time rather than walked up front. A machine's disk is
 * not a fixture: `/` on a container has tens of thousands of entries under it,
 * and fetching the lot to draw a tree would be slow on Docker and a bill on
 * Daytona. Each folder is fetched when it is opened, and cached until the user
 * asks for it again.
 */

/** What is worth opening in the preview pane rather than describing. */
const TEXTUAL =
  /\.(md|txt|json|ya?ml|toml|ini|cfg|conf|log|js|jsx|mjs|cjs|ts|tsx|css|html|xml|sh|bash|py|rb|go|rs|java|c|h|cpp|sql|env|example|lock|gitignore|dockerfile)$/i;

/** Bigger than this and the preview asks first. Reading a 200MB log into a
 *  React state is a frozen window, not a preview. */
const PREVIEW_LIMIT = 400_000;

/**
 * The machine's root, not its working directory.
 *
 * This pane was rooted at `/workspace`, which on a fresh machine is empty - so
 * the first thing anyone saw was "This folder is empty" and the reasonable
 * conclusion was that the files pane did not work. A machine has a filesystem
 * and the pane should show it; `/workspace` is where the agent works, so it is
 * opened by default, but it is a folder in the tree rather than the ceiling.
 */
const ROOT = "/";

export function FilesPane({ computer }) {
  const { toast } = useToast();
  /** Opened on arrival, because it is where this machine's work lands. */
  const WORKDIR = workdirOf(computer);
  // The file open in the full-size viewer, if any.
  const [viewing, setViewing] = React.useState(null);

  /** Save a file from the machine onto the user's own disk. */
  const download = React.useCallback(
    async (row) => {
      try {
        const result = await machines.downloadFile(computer.id, row.path);
        if (result?.saved) {
          toast({ variant: "success", title: `Saved ${row.name}` });
        }
      } catch (error) {
        toast({
          variant: "error",
          title: `Could not download ${row.name}`,
          description: error?.message ?? String(error),
        });
      }
    },
    [computer.id, toast]
  );
  // path -> entries, so a folder that has been opened once is instant again.
  const [tree, setTree] = React.useState({});
  const [expanded, setExpanded] = React.useState(() => new Set([ROOT, WORKDIR]));
  const [loading, setLoading] = React.useState(() => new Set());
  const [failed, setFailed] = React.useState({});
  const [selected, setSelected] = React.useState(null);
  const listRef = React.useRef(null);

  const usable = computer.status === "running";

  const load = React.useCallback(
    async (path, { force = false } = {}) => {
      if (!usable) return;
      if (!force && tree[path]) return;
      setLoading((prev) => new Set(prev).add(path));
      try {
        const rows = await machines.listDir(computer.id, path);
        setTree((prev) => ({ ...prev, [path]: rows }));
        setFailed((prev) => {
          const next = { ...prev };
          delete next[path];
          return next;
        });
      } catch (error) {
        // A folder the agent cannot read is one line in the tree, not a dialog.
        // `/root` on a container is exactly this, and it is not a failure.
        setFailed((prev) => ({ ...prev, [path]: error?.message ?? String(error) }));
      } finally {
        setLoading((prev) => {
          const next = new Set(prev);
          next.delete(path);
          return next;
        });
      }
    },
    [computer.id, tree, usable]
  );

  // A different machine is a different filesystem.
  React.useEffect(() => {
    setTree({});
    setFailed({});
    setSelected(null);
    setExpanded(new Set([ROOT, WORKDIR]));
  }, [computer.id]);

  React.useEffect(() => {
    if (!usable) return;
    if (!tree[ROOT] && !loading.has(ROOT) && !failed[ROOT]) load(ROOT);
    if (!tree[WORKDIR] && !loading.has(WORKDIR) && !failed[WORKDIR]) load(WORKDIR);
  }, [usable, tree, loading, failed, load]);

  const rows = React.useMemo(() => visibleRows(ROOT, tree, expanded), [tree, expanded]);

  if (!usable) {
    return (
      <EmptyState
        className="min-h-0 flex-1"
        icon={Folder}
        title={`${computer.name} is ${computer.status}`}
        description="Start the machine to browse its files."
      />
    );
  }

  function toggle(path) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else {
        next.add(path);
        load(path);
      }
      return next;
    });
  }

  function focusRow(index) {
    listRef.current?.querySelectorAll("[data-row]")[index]?.focus({ preventScroll: false });
  }

  function onKeyDown(event, row, index) {
    const isDir = row.type === "dir";
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        focusRow(Math.min(index + 1, rows.length - 1));
        break;
      case "ArrowUp":
        event.preventDefault();
        focusRow(Math.max(index - 1, 0));
        break;
      case "ArrowRight":
        event.preventDefault();
        if (isDir && !expanded.has(row.path)) toggle(row.path);
        else focusRow(Math.min(index + 1, rows.length - 1));
        break;
      case "ArrowLeft": {
        event.preventDefault();
        if (isDir && expanded.has(row.path)) {
          toggle(row.path);
          break;
        }
        const at = rows.findIndex((r) => r.path === parentPath(row.path));
        if (at >= 0) focusRow(at);
        break;
      }
      case "Home":
        event.preventDefault();
        focusRow(0);
        break;
      case "End":
        event.preventDefault();
        focusRow(rows.length - 1);
        break;
      default:
        break;
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 lg:flex-row">
      <div className="flex min-h-0 w-full flex-1 flex-col gap-2 lg:w-[300px] lg:flex-none">
        <div className="flex shrink-0 items-center justify-between gap-2">
          <p className="truncate font-mono text-[11px] text-muted-foreground">{ROOT}</p>
          <IconButton
            size="sm"
            label="Reload"
            onClick={() => {
              setTree({});
              load(ROOT, { force: true });
            }}
          >
            <RotateCw />
          </IconButton>
        </div>

        <ScrollArea
          className="min-h-36 flex-1 rounded-2xl bg-card-darker p-1.5"
          viewportRef={listRef}
        >
          <div role="tree" aria-label={`Files on ${computer.name}`} className="flex flex-col">
            {failed[ROOT] ? (
              <p className="px-2 py-4 text-center text-[13px] text-muted-foreground">
                {failed[ROOT]}
              </p>
            ) : rows.length === 0 ? (
              <p className="px-2 py-4 text-center text-[13px] text-muted-foreground">
                {loading.has(ROOT) ? "Reading…" : "This folder is empty."}
              </p>
            ) : (
              rows.map((row, i) => {
                const isDir = row.type === "dir";
                const open = expanded.has(row.path);
                const active = row.path === selected;
                const tabStop = active || (i === 0 && !rows.some((r) => r.path === selected));
                // A folder has nothing to view or download; a file has both.
                const fileItems = [
                  {
                    icon: Eye,
                    label: "View",
                    onSelect: () => {
                      setSelected(row.path);
                      if (imageMime(row.name)) setViewing(row);
                    },
                  },
                  { icon: Download, label: "Download", onSelect: () => download(row) },
                ];
                return (
                  <ContextMenu key={row.path} items={isDir ? [] : fileItems}>
                    <button
                      data-row=""
                      type="button"
                      role="treeitem"
                      aria-expanded={isDir ? open : undefined}
                      aria-selected={active}
                      tabIndex={tabStop ? 0 : -1}
                      onKeyDown={(event) => onKeyDown(event, row, i)}
                      onClick={() => {
                        setSelected(row.path);
                        if (isDir) toggle(row.path);
                      }}
                      onDoubleClick={() => {
                        if (!isDir && imageMime(row.name)) setViewing(row);
                      }}
                      style={{ paddingLeft: `${row.depth * 12 + 6}px` }}
                      className={cn(
                        // Rows are keyed by path, so only the ones a fold just
                        // revealed are new enough to fade; the rest stay put.
                        "flex h-7 w-full animate-fade-in items-center gap-1.5 rounded-xl pr-2 text-left outline-none",
                        "transition-colors duration-150 ease-out",
                        "focus-visible:fill-nav",
                        active ? "fill-nav-active" : "hover:fill-nav"
                      )}
                    >
                      <ChevronRight
                        aria-hidden="true"
                        className={cn(
                          "size-3 shrink-0 text-muted-foreground transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
                          !isDir ? "invisible" : open ? "rotate-90" : ""
                        )}
                      />
                      {isDir ? (
                        open ? (
                          <FolderOpen
                            className="size-3.5 shrink-0 text-muted-foreground"
                            aria-hidden="true"
                          />
                        ) : (
                          <Folder
                            className="size-3.5 shrink-0 text-muted-foreground"
                            aria-hidden="true"
                          />
                        )
                      ) : (
                        <File
                          className="size-3.5 shrink-0 text-muted-foreground"
                          aria-hidden="true"
                        />
                      )}
                      <span
                        className={cn(
                          "min-w-0 flex-1 truncate text-[13px]",
                          active && "font-medium"
                        )}
                        title={row.path}
                      >
                        {row.name}
                      </span>
                      {loading.has(row.path) ? (
                        <span className="shrink-0 text-[10px] text-muted-foreground">…</span>
                      ) : failed[row.path] ? (
                        <span
                          className="shrink-0 text-[10px] text-muted-foreground"
                          title={failed[row.path]}
                        >
                          locked
                        </span>
                      ) : null}
                    </button>
                  </ContextMenu>
                );
              })
            )}
          </div>
        </ScrollArea>
      </div>

      <FilePreview
        computer={computer}
        path={selected}
        entry={rows.find((row) => row.path === selected) ?? null}
        onDownload={download}
        onView={setViewing}
      />

      <ImageViewer
        computer={computer}
        entry={viewing}
        onOpenChange={(next) => !next && setViewing(null)}
        onDownload={download}
      />
    </div>
  );
}

/**
 * An image on the machine, at full size, in a modal.
 *
 * The bytes are fetched fresh when it opens rather than handed down from the
 * preview, because the two are reached different ways - a double-click, a
 * right-click View, the preview's own button - and the modal owning its own
 * load means every one of them works the same.
 */
function ImageViewer({ computer, entry, onOpenChange, onDownload }) {
  const [src, setSrc] = React.useState(null);
  const [error, setError] = React.useState(null);
  const open = Boolean(entry);
  const mime = entry ? imageMime(entry.name) : null;

  React.useEffect(() => {
    setSrc(null);
    setError(null);
    if (!entry || !mime) return undefined;

    let alive = true;
    machines
      .readFileBytes(computer.id, entry.path)
      .then((base64) => alive && setSrc(`data:${mime};base64,${base64}`))
      .catch((problem) => alive && setError(problem?.message ?? String(problem)));
    return () => {
      alive = false;
    };
  }, [computer.id, entry, mime]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange} size="lg" showClose={false}>
      <div className="flex items-center gap-2 px-4 py-2.5">
        <ImageIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate text-[13px] text-foreground" title={entry?.path}>
          {entry?.name}
        </span>
        <Button size="xs" variant="subtle" onClick={() => entry && onDownload(entry)}>
          <Download className="size-3.5" aria-hidden="true" />
          Download
        </Button>
        <DialogClose className="ml-1" />
      </div>
      <div className="flex min-h-64 items-center justify-center overflow-auto bg-card-darker p-4">
        {error ? (
          <p className="max-w-96 text-center text-[13px] text-muted-foreground">{error}</p>
        ) : src ? (
          <img
            src={src}
            alt={entry?.name}
            className="max-h-[70vh] max-w-full rounded-lg object-contain"
          />
        ) : (
          <Spinner />
        )}
      </div>
    </Dialog>
  );
}

/** Depth-first over what has been fetched and is currently open. */
function visibleRows(path, tree, expanded, depth = 0) {
  const rows = [];
  for (const item of tree[path] ?? []) {
    const full = joinPath(path, item.name);
    rows.push({ ...item, path: full, depth });
    if (item.type === "dir" && expanded.has(full)) {
      rows.push(...visibleRows(full, tree, expanded, depth + 1));
    }
  }
  return rows;
}

function FilePreview({ computer, path, entry, onDownload, onView }) {
  const [content, setContent] = React.useState(null);
  const [image, setImage] = React.useState(null);
  const [error, setError] = React.useState(null);
  const [busy, setBusy] = React.useState(false);

  const isFile = entry?.type === "file";
  const mime = isFile ? imageMime(entry.name) : null;
  const textual = isFile && !mime && (TEXTUAL.test(entry.name) || !entry.name.includes("."));
  const tooBig = isFile && (entry.size ?? 0) > PREVIEW_LIMIT;
  const imageTooBig = Boolean(mime) && (entry?.size ?? 0) > IMAGE_PREVIEW_LIMIT;
  const showImage = Boolean(mime) && !imageTooBig;

  React.useEffect(() => {
    setContent(null);
    setImage(null);
    setError(null);
    if (!isFile) return undefined;

    // An image is fetched as bytes and drawn; text is fetched as text. Anything
    // else is described, not read.
    if (showImage) {
      let alive = true;
      setBusy(true);
      machines
        .readFileBytes(computer.id, path)
        .then((base64) => alive && setImage(`data:${mime};base64,${base64}`))
        .catch((problem) => alive && setError(problem?.message ?? String(problem)))
        .finally(() => alive && setBusy(false));
      return () => {
        alive = false;
      };
    }

    if (!textual || tooBig) return undefined;

    let alive = true;
    setBusy(true);
    machines
      .readFile(computer.id, path)
      .then((text) => {
        if (alive) setContent(text);
      })
      .catch((problem) => {
        if (alive) setError(problem?.message ?? String(problem));
      })
      .finally(() => {
        if (alive) setBusy(false);
      });

    return () => {
      alive = false;
    };
  }, [computer.id, path, isFile, textual, tooBig, showImage, mime]);

  if (!entry) {
    return (
      <div className="flex min-h-40 min-w-0 flex-1 items-center justify-center rounded-2xl bg-card-darker px-6 text-center">
        <p className="text-[13px] text-muted-foreground">Select a file to read it.</p>
      </div>
    );
  }

  const crumbs = path.split("/").filter(Boolean);

  return (
    // Keyed on the path: a different file is a different page, and it fades
    // in as one rather than having its title, crumbs and body swapped in turn.
    <div
      key={path}
      className="flex min-h-40 min-w-0 flex-1 animate-fade-in flex-col gap-3 overflow-hidden rounded-2xl bg-card-darker p-4"
    >
      <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1">
        <span className="flex min-w-0 items-center gap-1.5 text-[15px] text-foreground">
          {entry.type === "dir" ? (
            <Folder className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          ) : mime ? (
            <ImageIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          ) : (
            <FileText className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          )}
          <span className="truncate">{entry.name}</span>
        </span>
        <span className="text-[11px] text-muted-foreground">
          {entry.type === "dir" ? "Directory" : formatBytes(entry.size ?? 0)}
          {entry.modifiedAt ? ` · ${relativeTime(entry.modifiedAt)}` : ""}
        </span>
        {isFile ? (
          <span className="ml-auto flex shrink-0 items-center gap-1">
            {mime ? (
              <IconButton size="sm" label="View full size" onClick={() => onView?.(entry)}>
                <Eye />
              </IconButton>
            ) : null}
            <IconButton
              size="sm"
              label={`Download ${entry.name}`}
              onClick={() => onDownload?.(entry)}
            >
              <Download />
            </IconButton>
          </span>
        ) : null}
      </div>

      <nav
        aria-label="Path"
        className="flex shrink-0 flex-wrap items-center gap-1 text-[11px] text-muted-foreground"
      >
        {crumbs.map((crumb, i) => (
          <span key={`${crumb}-${i}`} className="flex items-center gap-1">
            {i > 0 ? <ChevronRight className="size-3" aria-hidden="true" /> : null}
            <span className={cn(i === crumbs.length - 1 && "text-foreground")}>{crumb}</span>
          </span>
        ))}
      </nav>

      {entry.type === "dir" ? (
        <p className="text-[13px] text-muted-foreground">
          A directory. Open it in the tree to see what is inside.
        </p>
      ) : error ? (
        <p className="text-[13px] text-muted-foreground">{error}</p>
      ) : busy ? (
        <p className="flex items-center gap-2 text-[13px] text-muted-foreground">
          <Spinner className="size-4" /> Reading from {computer.name}…
        </p>
      ) : showImage ? (
        <button
          type="button"
          onClick={() => onView?.(entry)}
          title="View full size"
          className="grid min-h-24 flex-1 place-items-center overflow-auto rounded-xl fill-whisper p-3 outline-none focus-visible:fill-control-hover"
        >
          {image ? (
            <img
              src={image}
              alt={entry.name}
              className="max-h-[52vh] max-w-full rounded-lg object-contain"
            />
          ) : null}
        </button>
      ) : imageTooBig ? (
        <p className="text-[13px] text-muted-foreground">
          {formatBytes(entry.size)} is large to draw here. Download it, or open it full size.
        </p>
      ) : tooBig ? (
        <p className="text-[13px] text-muted-foreground">
          {formatBytes(entry.size)} is too large to preview. Read it on the machine with the
          terminal instead, or download it.
        </p>
      ) : !textual ? (
        <p className="text-[13px] text-muted-foreground">
          No preview for this file type - {formatBytes(entry.size ?? 0)}. Use Download to save it.
        </p>
      ) : (
        <pre className="no-scrollbar min-h-24 flex-1 overflow-auto rounded-xl fill-whisper p-3 font-mono text-[12.5px] leading-relaxed text-muted-foreground">
          {content || "(empty file)"}
        </pre>
      )}
    </div>
  );
}
