import * as React from "react";
import { ChevronRight, ExternalLink } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { CodeBlock } from "@/features/chat/markdown/CodeBlock";
import { FilePreview } from "@/features/chat/FilePreview";
import { useSwap } from "@/features/chat/arrival";
import { openLink } from "@/features/chat/markdown/Inline";
import {
  attachmentUrl,
  cardOf,
  classify,
  columnsOf,
  fitsATable,
  isImageUrl,
  isUrl,
  labelOf,
  leadOf,
  parseResult,
  preview,
  roleOf,
  toneOf,
} from "./mcp-result.js";

/**
 * The body of a tool nobody here wrote.
 *
 * An MCP server answers with JSON and the card used to print it: one monospace
 * line, a thousand characters wide, scrolling off the right edge of the
 * transcript. Everything was in it and none of it could be read.
 *
 * So the result draws itself according to its shape - a list of results as
 * cards, a uniform list as a table, a picture as a picture, a record as fields
 * with the long ones folded away. No server is named anywhere in here and no
 * tool is special-cased; the rules in `mcp-result.js` look at the value and
 * nothing else, which is the only way this can work for a server that will be
 * written next year.
 *
 * Two promises hold it together. Nothing is dropped: a field nobody recognised
 * renders as its key and its value, and a shape nobody expected renders as
 * JSON. And nothing is hidden for good: the raw result is one click away on
 * every card, so a reader who suspects the pretty version of lying can check.
 */

const CLAMP = "line-clamp-3";
const ROW = "flex min-w-0 gap-3 py-1";
const KEY = "w-28 shrink-0 pt-px text-[11px] text-muted-foreground";
const VAL = "min-w-0 flex-1 text-[12px] leading-relaxed break-words text-foreground";

/** Past this a value is a document, and a document gets a code block. */
const MAX_DEPTH = 4;
const LIST_CAP = 12;
const TABLE_CAP = 25;

/** Show the first few, then say how many are left. Used by every list here. */
function useCapped(items, cap) {
  const [all, setAll] = React.useState(false);
  const shown = all ? items : items.slice(0, cap);
  const rest = items.length - shown.length;
  const more = rest > 0 ? (
    <Button variant="ghost" size="xs" onClick={() => setAll(true)}>
      {`Show ${rest} more`}
    </Button>
  ) : null;
  return [shown, more];
}

/* -- leaves --------------------------------------------------------------- */

function Link({ href, children }) {
  return (
    <a
      href={href}
      onClick={(event) => openLink(event, href)}
      className="inline-flex min-w-0 items-baseline gap-1 text-foreground underline decoration-muted-foreground/50 underline-offset-4 hover:decoration-foreground"
    >
      <span className="min-w-0 truncate">{children ?? href}</span>
      <ExternalLink aria-hidden="true" className="size-3 shrink-0 translate-y-px text-muted-foreground" />
    </a>
  );
}

/**
 * A picture from a result, at a size that leaves room for the conversation.
 *
 * Clicking opens it full size, the same as every other picture in the app. An
 * image that will not load takes its frame with it rather than leaving the
 * browser's broken-image icon in the transcript - the URL is still in the raw
 * result, and a wrong guess about a field should cost nothing.
 */
function Picture({ url, name, className }) {
  const [open, setOpen] = React.useState(false);
  const [broken, setBroken] = React.useState(false);
  if (!url || broken) return null;
  const label = name || String(url).split(/[\\/]/).pop()?.slice(0, 60) || "image";

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className={cn(
          "overflow-hidden rounded-xl border border-border-subtle transition-[border-color] hover:border-border-strong",
          className
        )}
      >
        <img
          src={url}
          alt={label}
          onError={() => setBroken(true)}
          className="max-h-64 w-auto max-w-full object-contain"
        />
      </button>
      <FilePreview
        file={open ? { name: label, kind: "image", dataUrl: url } : null}
        onClose={() => setOpen(false)}
      />
    </>
  );
}

/** Long prose, clamped until asked for. */
function Prose({ text }) {
  const [full, setFull] = React.useState(false);
  const long = text.length > 400 || text.split("\n").length > 6;
  return (
    <div className="flex flex-col items-start gap-1">
      <p
        className={cn(
          "text-[12px] leading-relaxed whitespace-pre-wrap break-words text-foreground",
          long && !full && CLAMP
        )}
      >
        {text}
      </p>
      {long ? (
        <Button variant="ghost" size="xs" onClick={() => setFull((v) => !v)}>
          {full ? "Show less" : "Show more"}
        </Button>
      ) : null}
    </div>
  );
}

/** A scalar, coloured only when its meaning earns it. */
function Scalar({ name, value }) {
  const tone = toneOf(name, value);
  if (tone !== "neutral") {
    return (
      <Badge variant={tone} size="sm">
        {String(value)}
      </Badge>
    );
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return <span className="font-mono text-[12px] tabular-nums text-foreground">{String(value)}</span>;
  }
  return <span className="text-[12px] text-foreground">{String(value)}</span>;
}

/* -- folds ---------------------------------------------------------------- */

/**
 * A named section that can be put away.
 *
 * Open by default only where the reader is going to look anyway - the top
 * level, and the one collection that is obviously the answer. Everything
 * deeper starts folded with its size on the label, so a result with six nested
 * objects is six lines until somebody wants more.
 */
function Fold({ label, hint, defaultOpen = false, children }) {
  const [open, setOpen] = React.useState(defaultOpen);
  return (
    <div className="min-w-0">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex h-7 w-full items-center gap-1.5 rounded-lg px-1 text-left outline-none transition-colors duration-150 ease-out hover:fill-nav focus-visible:fill-nav"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn(
            "size-3.5 shrink-0 text-muted-foreground transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
            open && "rotate-90"
          )}
        />
        <span className="shrink-0 text-[12px] text-foreground">{label}</span>
        {hint ? <span className="min-w-0 truncate text-[11px] text-muted-foreground">{hint}</span> : null}
      </button>
      <Collapse open={open} innerClassName="min-w-0 pb-1 pl-5">
        {children}
      </Collapse>
    </div>
  );
}

/* -- collections ---------------------------------------------------------- */

/** A list of plain values: pictures if that is what they are, chips otherwise. */
function List({ items, name, depth }) {
  const [shown, more] = useCapped(items, LIST_CAP);

  if (items.every(isImageUrl)) {
    return (
      <div className="flex flex-col gap-1.5">
        <div className="flex flex-wrap gap-2">
          {shown.map((url, i) => (
            <Picture key={i} url={url} className="max-w-[46%]" />
          ))}
        </div>
        {more}
      </div>
    );
  }

  if (items.every((item) => item === null || typeof item !== "object")) {
    return (
      <div className="flex flex-col gap-1.5">
        <ul className="flex flex-col gap-0.5">
          {shown.map((item, i) => (
            <li key={i} className="flex min-w-0 gap-2 text-[12px] leading-relaxed text-foreground">
              <span aria-hidden="true" className="shrink-0 text-muted-foreground/60">
                ·
              </span>
              <span className="min-w-0 break-words">
                {isUrl(item) ? (
                  <Link href={item} />
                ) : (
                  <Scalar name={name} value={item} />
                )}
              </span>
            </li>
          ))}
        </ul>
        {more}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-col gap-1.5">
        {shown.map((item, i) => (
          <Card key={i} row={item} depth={depth} />
        ))}
      </div>
      {more}
    </div>
  );
}

/** Rows that agree about their columns, side by side so they can be compared. */
function Table({ rows }) {
  const columns = columnsOf(rows);
  const [shown, more] = useCapped(rows, TABLE_CAP);

  return (
    <div className="flex flex-col gap-1.5">
      <div className="no-scrollbar overflow-x-auto rounded-xl fill-whisper">
        <table className="w-full min-w-max text-left">
          <thead>
            <tr>
              {columns.map((key) => (
                <th
                  key={key}
                  className="px-3 py-1.5 text-[11px] font-medium whitespace-nowrap text-muted-foreground"
                >
                  {labelOf(key)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {shown.map((row, i) => (
              <tr key={i} className="border-t border-border-subtle/60">
                {columns.map((key) => {
                  const value = row?.[key];
                  return (
                    <td key={key} className="max-w-[22rem] px-3 py-1.5 align-top">
                      {value == null || value === "" ? (
                        <span className="text-[12px] text-muted-foreground/50">—</span>
                      ) : roleOf(key, value) === "url" ? (
                        <Link href={value} />
                      ) : (
                        <Scalar name={key} value={value} />
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {more}
    </div>
  );
}

/**
 * One object out of a list, as the result it probably is.
 *
 * Title, link, picture, paragraph - in that order, because that is the order a
 * person reads a search hit, a file, a message or a row from a database. What
 * is left keeps its own name underneath.
 */
function Card({ row, depth = 0 }) {
  if (row === null || typeof row !== "object") {
    return <Scalar name="" value={row} />;
  }
  if (Array.isArray(row)) {
    return <Value value={row} depth={depth + 1} />;
  }

  const card = cardOf(row);
  const heading = card.title || card.url;

  return (
    <div className="flex min-w-0 gap-3 rounded-xl fill-whisper p-2.5">
      {card.image ? (
        <Picture url={card.image} className="size-14 shrink-0 [&_img]:size-14 [&_img]:object-cover" />
      ) : null}
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        {heading ? (
          <div className="min-w-0 text-[13px] leading-snug font-medium text-foreground">
            {card.url ? <Link href={card.url}>{card.title || card.url}</Link> : card.title}
          </div>
        ) : null}
        {card.body ? (
          <p className={cn("text-[12px] leading-relaxed break-words text-muted-foreground", CLAMP)}>
            {card.body}
          </p>
        ) : null}
        {card.fields.length ? (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            {card.fields.map(([key, value]) => (
              <span key={key} className="inline-flex min-w-0 items-baseline gap-1.5">
                <span className="text-[11px] text-muted-foreground">{labelOf(key)}</span>
                <Scalar name={key} value={value} />
              </span>
            ))}
          </div>
        ) : null}
        {card.nested.map(([key, value]) => (
          <Fold key={key} label={labelOf(key)} hint={preview(value)}>
            <Value value={value} depth={depth + 1} />
          </Fold>
        ))}
      </div>
    </div>
  );
}

/**
 * An object, as fields.
 *
 * The long text field, when there is one, is lifted to the top: a search server
 * answers with its answer and then its sources, and the answer is what the
 * reader came for. Short fields become rows, collections become folds, and the
 * one collection that is plainly the point of the result opens itself.
 */
function Record({ record, depth }) {
  const entries = Object.entries(record).filter(
    ([, value]) => value != null && value !== "" && !(Array.isArray(value) && !value.length)
  );
  const lead = depth === 0 ? leadOf(record) : "";
  const biggest = depth > MAX_DEPTH ? null : widestCollection(entries);

  return (
    <div className="flex min-w-0 flex-col gap-1">
      {lead ? <Prose text={record[lead]} /> : null}
      {entries.map(([key, value]) => {
        if (key === lead) return null;
        const shape = classify(value);

        if (shape === "image") {
          return <Picture key={key} url={value} name={labelOf(key)} />;
        }
        if (shape === "record" || shape === "list" || shape === "table" || shape === "text") {
          return (
            <Fold
              key={key}
              label={labelOf(key)}
              hint={preview(value)}
              defaultOpen={key === biggest || (depth === 0 && entries.length === 1)}
            >
              <Value value={value} depth={depth + 1} />
            </Fold>
          );
        }
        return (
          <div key={key} className={ROW}>
            <span className={KEY}>{labelOf(key)}</span>
            <span className={VAL}>
              {roleOf(key, value) === "url" ? (
                <Link href={value} />
              ) : (
                <Scalar name={key} value={value} />
              )}
            </span>
          </div>
        );
      })}
    </div>
  );
}

/** The collection a record is actually about: the longest list it holds. */
function widestCollection(entries) {
  let best = null;
  let size = 0;
  for (const [key, value] of entries) {
    if (!Array.isArray(value) || value.length <= size) continue;
    best = key;
    size = value.length;
  }
  return best;
}

/** Any value, drawn as whatever it turned out to be. */
function Value({ value, depth = 0 }) {
  if (depth > MAX_DEPTH) {
    return <CodeBlock lang="json" code={JSON.stringify(value, null, 2)} />;
  }

  switch (classify(value)) {
    case "empty":
      return <p className="text-[12px] text-muted-foreground">Nothing.</p>;
    case "image":
      return <Picture url={value} />;
    case "text":
      return <Prose text={String(value)} />;
    case "scalar":
      return <Scalar name="" value={value} />;
    case "table":
      return fitsATable(value) ? <Table rows={value} /> : <List items={value} depth={depth} />;
    case "list":
      return <List items={value} depth={depth} />;
    case "record":
      return <Record record={value} depth={depth} />;
    default:
      return null;
  }
}

/* -- the card body -------------------------------------------------------- */

/** Pictures and sounds the server sent as content rather than as text. */
function Attachments({ attachments }) {
  const media = attachments.filter((one) => one?.type === "image" || one?.type === "audio");
  if (!media.length) return null;

  return (
    <div className="flex flex-wrap gap-2">
      {media.map((one, i) => {
        const url = attachmentUrl(one);
        if (!url) return null;
        if (one.type === "audio") {
          return <audio key={i} src={url} controls className="w-full max-w-sm" />;
        }
        return <Picture key={i} url={url} className="max-w-[46%]" />;
      })}
    </div>
  );
}

export function McpResult({ output, metadata, fallback }) {
  const text = typeof output === "string" ? output : "";
  const data = React.useMemo(() => parseResult(text), [text]);
  const [raw, setRaw] = React.useState(false);
  const attachments = metadata?.attachments ?? [];

  const pretty = React.useMemo(() => {
    if (data === undefined) return text;
    try {
      return JSON.stringify(data, null, 2);
    } catch {
      return text;
    }
  }, [data, text]);

  // The pretty result and the raw one are the same thing shown two ways, so
  // switching between them is a cross-fade rather than a replacement.
  const swap = useSwap(raw);

  return (
    <div className="flex min-w-0 flex-col gap-2">
      {attachments.length ? <Attachments attachments={attachments} /> : null}

      <div
        key={raw ? "raw" : "pretty"}
        onAnimationEnd={swap.onAnimationEnd}
        className={cn("flex min-w-0 flex-col gap-2", swap.swapping && "animate-fade-in")}
      >
        {data === undefined || raw ? null : <Value value={data} depth={0} />}
        {data === undefined && !raw ? fallback : null}
        {raw ? <CodeBlock lang="json" code={pretty} /> : null}
      </div>

      <div className="flex items-center gap-2">
        {metadata?.truncated ? (
          <Badge variant="warning" size="sm">
            truncated
          </Badge>
        ) : null}
        {text.trim() ? (
          <Button variant="ghost" size="xs" onClick={() => setRaw((v) => !v)}>
            {raw ? "Show result" : "Show raw"}
          </Button>
        ) : null}
      </div>
    </div>
  );
}
