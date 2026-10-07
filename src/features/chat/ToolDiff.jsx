import * as React from "react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

/**
 * A unified diff, rendered.
 *
 * This is the component a person actually reads before they let an agent write
 * to their disk, so it is built to be trusted rather than to be pretty: real
 * line numbers from the `@@` header, the sign column kept, and nothing
 * reflowed. A wrapped diff line is a lie about what the file will contain, so
 * the block scrolls sideways and the page never does.
 *
 * Parsing is deliberately forgiving. A model can hand us a diff with no
 * header, a truncated one, or plain text that was never a diff at all - and in
 * every one of those cases showing the raw text is more useful than an error,
 * because the reader still has to decide.
 */

const MONO = "font-mono text-[12px] leading-[1.55] whitespace-pre";
const HUNK_CAP = 3;

const HUNK_HEADER = /^@@+\s+-(\d+)(?:,\d+)?\s+\+(\d+)(?:,\d+)?\s+@@+/;

/** Split a unified diff into hunks, numbering every row as we go. */
export function parseDiff(text) {
  const source = typeof text === "string" ? text : Array.isArray(text) ? text.join("\n") : "";
  if (!source.trim()) return { hunks: [], raw: source };

  const hunks = [];
  let hunk = null;
  let oldNo = 0;
  let newNo = 0;

  for (const line of source.split("\n")) {
    if (line.startsWith("@@")) {
      const match = HUNK_HEADER.exec(line);
      oldNo = match ? Number(match[1]) : 1;
      newNo = match ? Number(match[2]) : 1;
      hunk = { header: line, rows: [] };
      hunks.push(hunk);
      continue;
    }
    // Everything before the first hunk is file headers (---, +++, index, diff
    // --git). They carry no line the reader can act on, so they are dropped.
    if (!hunk) continue;

    if (line.startsWith("\\")) {
      hunk.rows.push({ kind: "note", text: line.slice(1).trim() });
    } else if (line.startsWith("+")) {
      hunk.rows.push({ kind: "add", text: line.slice(1), newNo: newNo++ });
    } else if (line.startsWith("-")) {
      hunk.rows.push({ kind: "del", text: line.slice(1), oldNo: oldNo++ });
    } else {
      hunk.rows.push({
        kind: "ctx",
        text: line.startsWith(" ") ? line.slice(1) : line,
        oldNo: oldNo++,
        newNo: newNo++,
      });
    }
  }

  return { hunks, raw: source };
}

/** Added / removed / unchanged, for the row and for the screen reader. */
const ROW_LABEL = { add: "added", del: "removed", ctx: "unchanged", note: "note" };

function Gutter({ value }) {
  return (
    <span className="w-8 shrink-0 select-none pr-2 text-right text-muted-foreground/60 tabular-nums">
      {value ?? ""}
    </span>
  );
}

function Row({ row }) {
  const sign = row.kind === "add" ? "+" : row.kind === "del" ? "-" : " ";
  return (
    <div
      className={cn(
        MONO,
        "flex min-w-max",
        row.kind === "add" && "bg-success-wash text-success-ink",
        row.kind === "del" && "bg-destructive-wash text-destructive-ink",
        row.kind === "ctx" && "text-muted-foreground",
        row.kind === "note" && "text-muted-foreground/70 italic"
      )}
    >
      <Gutter value={row.oldNo} />
      <Gutter value={row.newNo} />
      <span className="sr-only">{ROW_LABEL[row.kind]} </span>
      <span aria-hidden="true" className="w-3 shrink-0 select-none">
        {row.kind === "note" ? "" : sign}
      </span>
      <span>{row.text || " "}</span>
    </div>
  );
}

export function ToolDiff({ diff, className }) {
  const parsed = React.useMemo(() => parseDiff(diff), [diff]);
  const [showAll, setShowAll] = React.useState(false);

  // Not a diff, or an empty one. Show whatever we were given rather than an
  // empty box - a reader deciding on a write needs the evidence either way.
  if (!parsed.hunks.length) {
    if (!parsed.raw.trim()) return null;
    return (
      <div className={cn("no-scrollbar overflow-x-auto rounded-xl fill-whisper p-3", className)}>
        <div className={cn(MONO, "min-w-max text-muted-foreground")}>{parsed.raw}</div>
      </div>
    );
  }

  const shown = showAll ? parsed.hunks : parsed.hunks.slice(0, HUNK_CAP);
  const hidden = parsed.hunks.length - shown.length;

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <div className="no-scrollbar overflow-x-auto rounded-xl fill-whisper py-2">
        {shown.map((hunk, i) => (
          <div key={i}>
            {/* The @@ line is context, not content - dimmed, and given air above
                so two hunks never look like one continuous run of code. */}
            <div className={cn(MONO, "min-w-max px-3 text-muted-foreground/60", i > 0 && "mt-2")}>
              {hunk.header}
            </div>
            <div className="px-3">
              {hunk.rows.map((row, j) => (
                <Row key={j} row={row} />
              ))}
            </div>
          </div>
        ))}
      </div>
      {hidden > 0 ? (
        <Button variant="ghost" size="xs" className="self-start" onClick={() => setShowAll(true)}>
          {`Show ${hidden} more ${hidden === 1 ? "hunk" : "hunks"}`}
        </Button>
      ) : null}
    </div>
  );
}
