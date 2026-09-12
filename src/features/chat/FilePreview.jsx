import * as React from "react";
import { Download, ExternalLink, FileText, Image as ImageGlyph, X } from "@/components/icons";
import { IconButton } from "@/components/ui/icon-button";
import { Portal, useEscapeLayer } from "@/components/ui/portal";
import { cn } from "@/lib/utils";
import { usePresence } from "@/hooks/use-presence";
import { useToast } from "@/components/ui/toast";
import { formatBytes } from "./attachments";
import { CodeBlock } from "./markdown/CodeBlock.jsx";

/**
 * A file in a message, looked at properly.
 *
 * The transcript can only afford a thumbnail or a pill for each attachment, so
 * this is where the whole thing goes: an image at the size the window allows,
 * a text file with its syntax coloured. It is deliberately not a route or a
 * screen - opening a file should not lose your place in the conversation - so
 * it is an overlay that closes on Escape and on a click outside it.
 *
 * What it shows is what the message already carries. An attachment holds its
 * own bytes; a picture on disk is fetched once through the bridge, because the
 * window's content policy will not load a `file://` image.
 */

const REMOTE = /^(https?:)?\/\//i;

/** The three places a picture in a message can come from. */
function sourceKind(src) {
  const value = String(src ?? "");
  if (!value) return null;
  if (value.startsWith("data:") || value.startsWith("blob:")) return "inline";
  if (REMOTE.test(value)) return "remote";
  return "disk";
}

/** A `file://` URL as the path it names, which is what the bridge wants. */
export function pathOf(src) {
  const value = String(src ?? "");
  if (!value.startsWith("file:")) return value;
  try {
    return decodeURIComponent(new URL(value).pathname.replace(/^\/(?=[A-Za-z]:)/, ""));
  } catch {
    return value.replace(/^file:\/\/\/?/, "");
  }
}

/**
 * A picture, wherever it lives, as something the window can actually paint.
 *
 * Neither kind can be loaded by the window directly: the content policy allows
 * `self`, `data:` and `blob:` only, which is what stops any markup in any reply
 * from reaching any host on its own. So both go through the main process - the
 * workspace bridge for a file, an anonymous fetch for a URL - and come back as
 * bytes. A picture that cannot be read is `failed`, and every caller shows the
 * link instead, which is what the transcript did before it could show pictures
 * at all.
 */
/**
 * What has already been fetched, so scrolling a transcript is not a fetch.
 *
 * A message is unmounted and remounted every time it leaves and re-enters the
 * view, and without this every one of those would be another request to the
 * host - which is the behaviour that made remote images a link in the first
 * place. Bounded, and dropped oldest first: it holds pictures, and a
 * transcript can be very long.
 */
const CACHE = new Map();
const CACHE_LIMIT = 40;

function remember(key, value) {
  CACHE.set(key, value);
  while (CACHE.size > CACHE_LIMIT) CACHE.delete(CACHE.keys().next().value);
  return value;
}

export function useMessageImage(src, enabled = true) {
  const kind = sourceKind(src);
  const [state, setState] = React.useState(() => {
    if (kind === "inline") return { url: String(src), bytes: 0, failed: false };
    return CACHE.get(String(src)) ?? { url: null, bytes: 0, failed: false };
  });

  React.useEffect(() => {
    if (!enabled || !kind) return undefined;
    if (kind === "inline") {
      setState({ url: String(src), bytes: 0, failed: false });
      return undefined;
    }

    const key = String(src);
    const known = CACHE.get(key);
    if (known) {
      setState(known);
      return undefined;
    }

    let live = true;
    setState({ url: null, bytes: 0, failed: false });
    const request =
      kind === "remote"
        ? window.electronAPI?.fetchImage?.(key)
        : window.workspaceAPI?.readImage?.(pathOf(src));

    Promise.resolve(request)
      .then((result) => {
        const next = result?.url
          ? { url: result.url, bytes: result.bytes ?? 0, failed: false }
          : { url: null, bytes: 0, failed: true };
        remember(key, next);
        if (live) setState(next);
      })
      .catch(() => {
        const next = { url: null, bytes: 0, failed: true };
        remember(key, next);
        if (live) setState(next);
      });
    return () => {
      live = false;
    };
  }, [src, kind, enabled]);

  return state;
}

/** The older name, kept for the callers that only ever have a path. */
export function useLocalImage(path, enabled = true) {
  return useMessageImage(path, enabled);
}

/**
 * Save this picture somewhere the person chooses.
 *
 * The bytes are already here - every picture in a message is a data URL by the
 * time it is drawn - so this is a dialog and a write, with no second request
 * to anywhere. Exported because both the picture in the message and the
 * picture filling the screen offer it, and they should behave identically.
 */
export function useSaveImage(url, name) {
  const { toast } = useToast();

  return React.useCallback(
    async (event) => {
      event?.stopPropagation();
      event?.preventDefault();
      if (!url) return;
      const result = await window.electronAPI?.saveImage?.({ dataUrl: url, name });
      if (result?.saved) toast({ title: "Picture saved", description: result.path, variant: "success" });
      else if (result?.error) toast({ title: "It could not be saved", description: result.error, variant: "danger" });
    },
    [url, name, toast]
  );
}

/** The button itself, over the top-right corner of whatever it is saving. */
export function SaveImageButton({ url, name, className }) {
  const save = useSaveImage(url, name);
  if (!window.electronAPI?.saveImage) return null;
  return (
    <span className={cn("absolute right-1.5 top-1.5", className)}>
      <IconButton
        size="sm"
        label="Save this picture"
        onClick={save}
        className="bg-black/45 text-white backdrop-blur-sm hover:bg-black/65 hover:text-white"
      >
        <Download />
      </IconButton>
    </span>
  );
}

function Chrome({ title, subtitle, state, onClose, actions, children }) {
  const ref = React.useRef(null);
  useEscapeLayer(true, onClose);

  return (
    <Portal>
      <div
        data-state={state}
        className="fixed inset-0 z-100 flex items-center justify-center scrim p-6 animate-fade-in"
        onMouseDown={(event) => {
          // Only a click that starts outside closes it, so a drag that ends on
          // the backdrop while selecting text does not.
          if (!ref.current?.contains(event.target)) onClose();
        }}
      >
        <div
          ref={ref}
          role="dialog"
          aria-modal="true"
          aria-label={title}
          data-state={state}
          className="flex max-h-full min-h-0 w-full max-w-4xl flex-col overflow-hidden rounded-3xl overlay-surface animate-overlay-in"
        >
          <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border-subtle pl-4 pr-2">
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm text-foreground">{title}</div>
              {subtitle ? (
                <div className="truncate text-[11px] text-muted-foreground">{subtitle}</div>
              ) : null}
            </div>
            {actions}
            <IconButton label="Close" onClick={onClose}>
              <X />
            </IconButton>
          </div>
          <div className="min-h-0 flex-1 overflow-auto p-4">{children}</div>
        </div>
      </div>
    </Portal>
  );
}

/**
 * A picture, with nothing around it.
 *
 * No panel, no title bar, no filename: the picture is the content, and a card
 * behind it only makes the window smaller and puts a grey border between the
 * reader and the thing they clicked. Just the image on the dimmed page, with
 * the close in the corner of the screen where every full-screen viewer puts
 * it - which also means the controls never move when the picture is a tall one
 * rather than a wide one.
 */
function Lightbox({ url, alt, failed, state, onClose, onReveal }) {
  const ref = React.useRef(null);
  useEscapeLayer(true, onClose);

  return (
    <Portal>
      <div
        data-state={state}
        className="fixed inset-0 z-100 flex items-center justify-center scrim p-8 animate-fade-in"
        role="dialog"
        aria-modal="true"
        aria-label={alt || "Image"}
        onMouseDown={(event) => {
          // Anywhere but the picture itself closes it, and only when the click
          // started there - so dragging off the image does not.
          if (!ref.current?.contains(event.target)) onClose();
        }}
      >
        {/* No save here: the picture in the message carries that button on
            hover, and offering it twice put two icons in a corner that should
            hold one thing - the way out. */}
        <div className="absolute right-3 top-3 flex items-center gap-1">
          {onReveal ? (
            <IconButton label="Show it in the folder" onClick={onReveal}>
              <ExternalLink />
            </IconButton>
          ) : null}
          <IconButton label="Close" onClick={onClose}>
            <X />
          </IconButton>
        </div>

        {url ? (
          <img
            ref={ref}
            src={url}
            alt={alt || ""}
            data-state={state}
            className="max-h-full max-w-full rounded-xl object-contain animate-overlay-in"
          />
        ) : (
          <p className="text-sm text-muted-foreground">
            {failed ? "That image could not be read." : "Loading..."}
          </p>
        )}
      </div>
    </Portal>
  );
}

/** The language a filename suggests, for colouring a text preview. */
function languageFor(name) {
  const ext = String(name ?? "").split(".").pop()?.toLowerCase() ?? "";
  return ext === name ? "" : ext;
}

/**
 * Given a file, it opens; given nothing, it closes - and takes a beat to go.
 *
 * Callers keep it mounted and hand it `null` to close, rather than unmounting
 * it, because an element React has removed cannot be watched leaving. The
 * file that was open is kept for that beat so the exit has something to draw.
 */
export function FilePreview({ file: wanted, onClose }) {
  const { mounted, state } = usePresence(Boolean(wanted));
  const last = React.useRef(wanted);
  if (wanted) last.current = wanted;
  const file = mounted ? last.current : null;

  const isImage = file?.kind === "image";
  const needsFetching = !file?.dataUrl && !file?.text && (file?.src || file?.path);
  const disk = useMessageImage(needsFetching ? (file.src ?? file.path) : null, Boolean(needsFetching) && isImage);
  const url = file?.dataUrl ?? disk.url;

  if (!file) return null;

  const size = file.size ?? file.sizeBytes ?? disk.bytes;
  // Only a file with a path can be shown in a folder; an attachment lives in
  // the message and nowhere else.
  const reveal = file.path ? () => window.electronAPI?.showInFolder?.(file.path) : null;

  // A picture gets the bare view; a text file keeps the panel, because a page
  // of code floating on a dimmed screen has no edge to read it against.
  if (isImage) {
    return (
      <Lightbox
        url={url}
        alt={file.name}
        failed={disk.failed}
        state={state}
        onClose={onClose}
        onReveal={reveal}
      />
    );
  }

  return (
    <Chrome
      title={file.name}
      subtitle={[file.path, size ? formatBytes(size) : null].filter(Boolean).join("  ")}
      state={state}
      onClose={onClose}
      actions={
        reveal ? (
          <IconButton label="Show it in the folder" onClick={reveal}>
            <ExternalLink />
          </IconButton>
        ) : null
      }
    >
      <CodeBlock lang={languageFor(file.name)} code={file.text ?? ""} />
    </Chrome>
  );
}

/**
 * The thumbnail or pill a file gets inside a message.
 *
 * An image is shown, small: a picture nobody can see is a filename with extra
 * steps. Everything else keeps the pill it had, because a CSV has no useful
 * small form.
 */
export function FileChip({ file, onOpen }) {
  const isImage = file.kind === "image";
  const disk = useMessageImage(!file.dataUrl ? (file.src ?? file.path) : null, isImage);
  const url = file.dataUrl ?? disk.url;

  if (isImage && url) {
    return (
      <button
        type="button"
        onClick={() => onOpen(file)}
        title={file.name}
        className="group/thumb relative overflow-hidden rounded-xl border border-border-subtle transition-[border-color] hover:border-border-strong"
      >
        <img src={url} alt={file.name} className="h-28 w-auto max-w-56 object-cover" />
        <span className="absolute inset-x-0 bottom-0 truncate bg-black/55 px-2 py-1 text-left text-[10px] text-white opacity-0 transition-opacity group-hover/thumb:opacity-100">
          {file.name}
        </span>
      </button>
    );
  }

  const Glyph = isImage ? ImageGlyph : file.kind === "text" ? FileText : Download;
  return (
    <button
      type="button"
      onClick={() => onOpen(file)}
      title={file.name}
      className={cn(
        "flex h-7 max-w-64 items-center gap-1.5 rounded-full bg-muted px-2.5 text-[11px] text-muted-foreground",
        "transition-colors hover:text-foreground"
      )}
    >
      <Glyph className="size-3.5 shrink-0" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate text-foreground">{file.name}</span>
      <span className="shrink-0 tabular-nums">
        {formatBytes(file.size ?? file.sizeBytes)}
      </span>
    </button>
  );
}
