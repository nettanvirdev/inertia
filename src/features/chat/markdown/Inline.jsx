import * as React from "react";
import { Image as ImageGlyph } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Tooltip } from "@/components/ui/tooltip";
import { MathSpan } from "./math/Math.jsx";
import { FilePreview, SaveImageButton, pathOf, useMessageImage } from "../FilePreview.jsx";
import { allowRemote, mayFetch, remoteHost } from "../remote-image.js";
import { Button } from "@/components/ui/button";
import { FootnoteNumbers } from "./footnotes.js";
import { useArrival } from "../arrival.js";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { useAppIfAny } from "@/lib/store";
import { slugOf } from "@shared/group";

/**
 * `@handle` in a message, drawn as the agent it names.
 *
 * The same pill the composer draws while the message is being written, so a
 * mention looks the same before and after it is sent; and a button, because
 * in any messaging app a name in a message is the way to the person. Clicking
 * it opens the agent. A handle that is nobody's stays as it was typed.
 */
function Mention({ handle }) {
  const app = useAppIfAny();
  const agents = app?.agents ?? [];
  const wanted = String(handle ?? "").toLowerCase();
  const agent = agents.find((one) => slugOf(one.name) === wanted) ?? null;
  if (!agent || !app) return <>@{handle}</>;
  return (
    <button
      type="button"
      className="prompt-pill cursor-pointer hover:brightness-110"
      data-kind="agent"
      title={`Open ${agent.name}`}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        app.setActiveAgentId(agent.id);
        app.setView("agents");
      }}
    >
      <AgentAvatar agent={agent} size="xs" className="prompt-pill-face" />
      <span>{agent.name}</span>
    </button>
  );
}

/**
 * Inline nodes to React elements. Every branch produces an element or a
 * string - nothing here ever touches `dangerouslySetInnerHTML`, so no amount
 * of markup in a model's reply can become markup on the page.
 */

export function openLink(event, href) {
  event.preventDefault();
  if (window.electronAPI?.openExternal) window.electronAPI.openExternal(href);
  else window.open(href, "_blank", "noopener,noreferrer");
}

const linkClass =
  "text-foreground underline decoration-muted-foreground/50 underline-offset-4 hover:decoration-foreground";

/**
 * Where a link actually goes, on hover.
 *
 * Not a fetch. Every other app's link preview loads the page to show a title
 * and a picture, which is a request to a host the model named, made because
 * someone's pointer passed over a word - the exact thing images are not
 * fetched for. Everything here is read out of the address itself, which is
 * what a reader needs anyway: the host, so a link whose text says one thing
 * and whose target says another is visible before it is clicked.
 */
function destination(href) {
  const raw = String(href ?? "");
  try {
    const url = new URL(raw.startsWith("www.") ? `https://${raw}` : raw);
    if (url.protocol === "mailto:") return { where: url.pathname, what: "Writes an email" };
    if (url.protocol === "file:") {
      return {
        where: decodeURIComponent(url.pathname.replace(/^\/(?=[A-Za-z]:)/, "")),
        what: "Opens on this machine",
      };
    }
    const rest = `${url.pathname}${url.search}`.replace(/\/$/, "");
    return {
      where: url.host,
      detail: rest && rest !== "/" ? rest : null,
      what: "Opens in your browser",
      insecure: url.protocol === "http:",
    };
  } catch {
    return { where: raw, what: null };
  }
}

function Link({ href, children }) {
  const target = React.useMemo(() => destination(href), [href]);
  return (
    <Tooltip
      side="top"
      className="h-auto max-w-80 flex-col items-start gap-0.5 py-1.5 leading-snug"
      content={
        <>
          <span className="font-medium">{target.where}</span>
          {target.detail ? (
            <span className="max-w-full truncate opacity-70">{target.detail}</span>
          ) : null}
          {target.what ? (
            <span className="opacity-70">
              {target.what}
              {target.insecure ? " over plain http" : ""}
            </span>
          ) : null}
        </>
      }
    >
      <a href={href} onClick={(event) => openLink(event, href)} className={linkClass}>
        {children}
      </a>
    </Tooltip>
  );
}

/**
 * The fallback: a picture that could not be loaded is the link it came from.
 *
 * A remote address that answered with something that is not an image, a file
 * that has been moved, a host that is not reachable - all of them end here
 * rather than as a broken element, and the reader can still open the address
 * themselves.
 */
function ImageLink({ src, alt }) {
  return (
    <a
      href={src}
      onClick={(event) => openLink(event, src)}
      className={cn(linkClass, "inline-flex items-baseline gap-1")}
      title={src}
    >
      <ImageGlyph className="size-3.5 translate-y-[0.15em] text-muted-foreground" />
      {alt || "image"}
    </a>
  );
}

/**
 * A picture in a message is shown, wherever it came from.
 *
 * The rule used to be that a remote image was a link, because loading one is a
 * request to a host the model named. It is still that request, so a remote
 * picture is drawn as the host it would come from until the reader clicks to
 * load it; then it is fetched by the Rust backend, anonymously and only from
 * the public internet - it refuses private and loopback addresses - and the
 * bytes come back as a data URL rather than the window reaching out. A picture
 * on this machine is simply shown. What the reader gets is the picture they
 * were promised, at a size that leaves the message readable, and the full
 * thing on a click.
 *
 * Anything that cannot be loaded falls back to the link it was: a broken
 * address in a reply reads as a broken link rather than as a hole.
 */
function MessageImage({ src, alt, width }) {
  // A picture from the internet waits for a click. Loading it is a request to
  // a host the model chose, and see remote-image.js for why that is the
  // person's call rather than the transcript's.
  const host = remoteHost(src);
  const [asked, setAsked] = React.useState(() => mayFetch(src));
  const image = useMessageImage(src, asked);
  const [open, setOpen] = React.useState(false);
  const name = alt || pathOf(src).split(/[\\/]/).pop() || "image";
  // A picture that was still being fetched when this mounted fades in over
  // the placeholder once it lands; one already in the cache is simply drawn,
  // because nothing was there for it to replace.
  const arrival = useArrival(null, { live: !image.url });

  if (!asked && !image.url) {
    return (
      <Button
        variant="secondary"
        size="sm"
        className="my-2 max-w-full"
        title={src}
        onClick={() => {
          allowRemote(src);
          setAsked(true);
        }}
      >
        <ImageGlyph />
        <span className="min-w-0 truncate">Load image from {host}</span>
      </Button>
    );
  }

  if (!image.url) {
    return image.failed ? (
      <ImageLink src={src} alt={alt} />
    ) : (
      <span className="my-2 flex h-24 w-40 animate-soft-pulse items-center justify-center rounded-xl fill-whisper text-[11px] text-muted-foreground">
        loading
      </span>
    );
  }

  return (
    <>
      {/* The frame is the positioning context for the save button, and a
          `group` so the button appears on hover over the picture rather than
          sitting on it permanently. */}
      <span
        onAnimationEnd={arrival.onAnimationEnd}
        className={cn(
          "group/picture relative my-2 block w-fit max-w-full",
          arrival.arriving && "animate-fade-in"
        )}
      >
        <button
          type="button"
          onClick={() => setOpen(true)}
          title={`${name} - click to see it full size`}
          // The width goes on the frame rather than on the picture. `fit-content`
          // on a box holding a replaced element measures that element's
          // *intrinsic* width, so a 1600px image asked to draw at 220 still gave
          // the border a 1600px box to hug.
          style={width ? { width: `min(100%, ${width}px)` } : undefined}
          className="block w-full overflow-hidden rounded-xl border border-border-subtle transition-[border-color] group-hover/picture:border-border-strong"
        >
          {/* The width a model asked for is honoured as a ceiling, never as a
              floor: `width="1600"` in a message column six hundred wide is not a
              request anyone would want granted. */}
          <img
            src={image.url}
            alt={alt || ""}
            className={cn("max-h-72 max-w-full object-contain", width ? "w-full" : "w-auto")}
          />
        </button>
        <SaveImageButton
          url={image.url}
          name={name}
          className="opacity-0 transition-opacity group-hover/picture:opacity-100 focus-within:opacity-100"
        />
      </span>
      <FilePreview
        file={
          open
            ? { name, kind: "image", dataUrl: image.url, src, path: host ? null : pathOf(src) }
            : null
        }
        onClose={() => setOpen(false)}
      />
    </>
  );
}

/** Tags that carry meaning of their own rather than a node type. */
const HTML_CLASS = {
  u: "underline underline-offset-4",
  mark: "rounded-sm bg-warning-wash px-0.5 text-foreground",
  small: "text-[0.85em] text-muted-foreground",
  kbd: "rounded border border-border-subtle fill-control px-1 py-px font-mono text-[0.8em]",
  samp: "font-mono text-[0.85em]",
  abbr: "underline decoration-dotted underline-offset-4",
  q: "before:content-['“'] after:content-['”']",
  big: "text-[1.15em]",
  span: "",
};

function Html({ node, children }) {
  if (node.tag === "sub") return <sub className="text-[0.75em]">{children}</sub>;
  if (node.tag === "sup") return <sup className="text-[0.75em]">{children}</sup>;
  return <span className={HTML_CLASS[node.tag] ?? ""}>{children}</span>;
}

/**
 * `[^1]`, as the number the note is written under.
 *
 * The numbering belongs to the whole message rather than to one paragraph, so
 * it comes from the context the markdown root provides. A reference with no
 * note under it keeps its own label - a model that wrote the marker and forgot
 * the text should look like that, not like a footnote that vanished.
 */
function FootRef({ id }) {
  const numbers = React.useContext(FootnoteNumbers);
  const number = numbers?.get(id);
  return (
    <sup
      className="ml-px text-[0.7em] text-muted-foreground"
      title={number ? `Note ${number}` : id}
    >
      [{number ?? id}]
    </sup>
  );
}

export function renderInline(nodes, keyPrefix = "i") {
  return nodes.map((node, index) => {
    const key = `${keyPrefix}-${index}`;
    switch (node.type) {
      case "strong":
        return (
          <strong key={key} className="font-medium text-foreground">
            {renderInline(node.children, key)}
          </strong>
        );
      case "em":
        return (
          <em key={key} className="italic">
            {renderInline(node.children, key)}
          </em>
        );
      case "del":
        return (
          <del key={key} className="text-muted-foreground line-through">
            {renderInline(node.children, key)}
          </del>
        );
      case "code":
        return (
          <code
            key={key}
            className="rounded-sm fill-control px-1 py-0.5 font-mono text-[0.85em] wrap-break-word"
          >
            {node.value}
          </code>
        );
      case "link":
        return (
          <Link key={key} href={node.href}>
            {renderInline(node.children, key)}
          </Link>
        );
      case "image":
        return <MessageImage key={key} src={node.src} alt={node.alt} width={node.width} />;
      case "html":
        return (
          <Html key={key} node={node}>
            {renderInline(node.children, key)}
          </Html>
        );
      case "footref":
        return <FootRef key={key} id={node.id} />;
      case "mention":
        return <Mention key={key} handle={node.handle} />;
      case "math":
        return <MathSpan key={key} value={node.value} />;
      case "break":
        return <br key={key} />;
      default:
        return <React.Fragment key={key}>{node.value}</React.Fragment>;
    }
  });
}

/**
 * The typing caret. An inline-block rather than a character so it sits on the
 * last line of the last block's text instead of dropping to a line of its own
 * the way a separate element after the prose would.
 */
export function Caret() {
  return (
    <span
      aria-hidden="true"
      className="ml-0.5 inline-block h-[0.95em] w-[0.42em] translate-y-[0.1em] animate-soft-pulse rounded-[1px] bg-foreground/70"
    />
  );
}
