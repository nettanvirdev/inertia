import * as React from "react";
import {
  Check,
  ChevronRight,
  ExternalLink,
  TriangleAlert,
  getIcon,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { formatDuration } from "@/data";
import { keyForTool, toolByKey } from "@shared/tools";
import { tools as agentTools } from "@/lib/agent";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { usePresence } from "@/hooks/use-presence";
import { ToolDiff } from "@/features/chat/ToolDiff";
import { CodeBlock } from "@/features/chat/markdown/CodeBlock";
import { PresentedFiles } from "@/features/chat/PresentedFiles";
import { TaskList } from "@/features/chat/TaskList";
import { PlanCard } from "@/features/chat/PlanCard";
import { FilePreview, useLocalImage } from "@/features/chat/FilePreview";
import { McpResult } from "@/features/chat/McpResult";
import { useArrival } from "@/features/chat/arrival";
import { GROUPED_TOOL, fileName, runCount, runDuration, runState, runTitle } from "@/features/chat/tool-groups";

/**
 * One tool call in the transcript.
 *
 * This is the row a reader sees more than any other once an agent can do
 * things, so the whole design is about scanning rather than reading: one line,
 * one height, never wrapping. Icon says which tool, title says what it touched,
 * the right edge says how it went. Ten of these stacked should be legible as a
 * shape - "read, read, edit, shell, shell" - without a single word being read.
 *
 * Collapsed by default, because a finished call is evidence rather than
 * content. Failed calls open themselves: that is the one state the reader
 * always wants, and making them click for it is making them click every time.
 *
 * The card is a surface step (`card-subtle`) with no border, as everywhere else
 * in the app - separation comes from the fill.
 */

const MONO = "font-mono text-[12px] leading-relaxed whitespace-pre";

/**
 * An opened card is a window onto its content, not the whole of it.
 *
 * A written file, a build log or a long read used to expand to its full height,
 * which for a 300-line component meant the rest of the conversation left the
 * screen and the reader had to scroll the transcript back to where they had
 * been. The height is a setting rather than a constant because how much is
 * worth seeing at once depends on the screen, and it is a variable rather than
 * a prop because nothing between here and the setting needs to know about it.
 */
/**
 * A ceiling, and deliberately no `overscroll-contain`.
 *
 * Containment stops a scroll chaining to the parent when the inner box reaches
 * its end, which is right for a panel laid over a page and wrong for a box
 * inside one: the wheel died at the bottom of whichever card the pointer
 * happened to be over, and the transcript behind it would not move until that
 * card was collapsed. Chaining is what "keep scrolling" means.
 */
const CARD_BODY = "overflow-y-auto max-h-[var(--tool-max-height,320px)]";
const OUTPUT_BOX = "no-scrollbar overflow-x-auto rounded-xl fill-whisper p-3";
/** The disclosure chevron's quarter turn, timed like every other toggle. */
const CHEVRON =
  "size-3.5 shrink-0 text-muted-foreground transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]";
const ARG_CAP = 200;
const OUTPUT_LINE_CAP = 20;

/** Tool id to glyph. The catalog knows most of them; these are the leaves. */
const ICON_BY_TOOL = {
  read: "FileText",
  ls: "FolderOpen",
  write: "FilePen",
  edit: "FilePen",
  glob: "Search",
  grep: "FileSearch",
  shell: "SquareTerminal",
  task: "Users",
  skill: "Sparkles",
  todowrite: "ListChecks",
  present_plan: "ListChecks",
  question: "MessageCircleQuestion",
};

function iconFor(name) {
  const direct = ICON_BY_TOOL[name];
  if (direct) return getIcon(direct);
  return getIcon(toolByKey(keyForTool(name))?.icon ?? "Wrench");
}

/** Tool output arrives as a string, but demo data and MCP servers send shapes. */
function asText(value) {
  if (value == null) return "";
  if (typeof value === "string") return value;
  if (Array.isArray(value)) return value.map(asText).join("\n");
  if (typeof value === "object") {
    if (Array.isArray(value.lines)) {
      return value.lines.map((l) => (typeof l === "string" ? l : (l?.text ?? ""))).join("\n");
    }
    if (typeof value.text === "string") return value.text;
    try {
      return JSON.stringify(value, null, 2);
    } catch {
      return String(value);
    }
  }
  return String(value);
}

function lastLine(text) {
  const lines = asText(text).split("\n").filter((l) => l.trim());
  return lines[lines.length - 1] ?? "";
}

const PATHISH = new Set(["read", "ls", "write", "edit", "glob", "grep"]);

/**
 * What a row says a tool did, where the permission group's name does not say it.
 *
 * The label on a row comes from the permission catalogue, because that is what
 * the reader agreed to when they allowed it. For most tools the two coincide.
 * For the file tools they do not: one key covers writing, editing, patching,
 * moving, copying and deleting, so every one of those rows read "Change files"
 * and a transcript of eleven new components looked like eleven edits to
 * something that already existed. The permission is still the group; the row
 * says which of its tools ran.
 */
const TOOL_LABEL = {
  write: "Write file",
  edit: "Edit file",
  patch: "Patch file",
  file_move: "Move file",
  file_copy: "Copy file",
  file_delete: "Delete file",
  file_folder: "New folder",
};

/** The fence tag for a file, from its name, so a preview is coloured. */
const LANG_BY_EXTENSION = {
  ts: "ts", tsx: "tsx", js: "js", jsx: "jsx", mjs: "js", cjs: "js",
  json: "json", css: "css", scss: "css", html: "html", md: "md", mdx: "md",
  py: "python", rs: "rust", go: "go", java: "java", rb: "ruby", php: "php",
  sh: "bash", bash: "bash", zsh: "bash", yml: "yaml", yaml: "yaml",
  toml: "toml", sql: "sql", c: "c", h: "c", cpp: "cpp", cs: "cs", swift: "swift",
};

function langFor(file) {
  const name = String(file ?? "").split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  if (dot < 0) return "";
  return LANG_BY_EXTENSION[name.slice(dot + 1).toLowerCase()] ?? "";
}

/**
 * A path's end is the part that identifies it. Truncating from the left is
 * done with direction, not by slicing, so the browser puts the ellipsis where
 * the overflow actually is at whatever width the pane happens to be; `bdi`
 * keeps the string itself reading left to right inside the flipped box.
 */
function Title({ text, fromLeft }) {
  if (!fromLeft) {
    return (
      <span className="min-w-0 flex-1 truncate text-[13px] text-foreground" title={text}>
        {text}
      </span>
    );
  }
  return (
    <span
      dir="rtl"
      className="min-w-0 flex-1 truncate text-left text-[13px] text-foreground"
      title={text}
    >
      <bdi>{text}</bdi>
    </span>
  );
}

/**
 * State, in text and in colour both. The running mark is a slow pulse rather
 * than a spinner: a spinner is the loudest thing on a still page, and there
 * can be five of these on screen at once.
 */
function StateMark({ state, durationMs }) {
  if (state === "running") {
    return (
      <span className="flex shrink-0 items-center gap-1.5 text-[11px] text-muted-foreground">
        <span aria-hidden="true" className="size-1.5 animate-pulse rounded-full bg-muted-foreground" />
        Running
      </span>
    );
  }
  if (state === "failed") {
    return (
      <span className="flex shrink-0 items-center gap-1 text-[11px] text-destructive-ink">
        <TriangleAlert className="size-3.5" aria-hidden="true" />
        Failed
      </span>
    );
  }
  return (
    <span className="flex shrink-0 items-center gap-1 text-[11px] tabular-nums text-muted-foreground">
      <Check className="size-3.5" aria-hidden="true" />
      <span className="sr-only">Done </span>
      {durationMs == null ? "" : formatDuration(durationMs)}
    </span>
  );
}

/** One argument. Long values stay one line until asked for. */
function ArgRow({ name, value }) {
  const text = typeof value === "string" ? value : asText(value);
  const long = text.length > ARG_CAP;
  const [full, setFull] = React.useState(false);
  return (
    <div className="flex gap-3 py-0.5">
      <span className="w-24 shrink-0 text-[11px] text-muted-foreground">{name}</span>
      <span className="min-w-0 flex-1 font-mono text-[12px] leading-relaxed break-words text-foreground">
        {long && !full ? `${text.slice(0, ARG_CAP)}…` : text}
        {long && !full ? (
          <button
            type="button"
            onClick={() => setFull(true)}
            className="ml-1.5 font-sans text-[11px] text-muted-foreground underline-offset-2 transition-colors duration-150 ease-out hover:text-foreground hover:underline"
          >
            show more
          </button>
        ) : null}
      </span>
    </div>
  );
}

function Args({ args, skip }) {
  const entries = Object.entries(args ?? {}).filter(
    ([key, value]) => !skip?.includes(key) && value != null && value !== ""
  );
  if (!entries.length) return null;
  return (
    <div className="flex flex-col">
      {entries.map(([key, value]) => (
        <ArgRow key={key} name={key} value={value} />
      ))}
    </div>
  );
}

/** The raw output, capped. A long build log should not own the transcript. */
function Output({ output, outputPath }) {
  const text = asText(output);
  const [full, setFull] = React.useState(false);
  if (!text.trim()) return null;

  const lines = text.split("\n");
  const capped = !full && lines.length > OUTPUT_LINE_CAP;
  const shown = capped ? lines.slice(0, OUTPUT_LINE_CAP) : lines;

  return (
    <div className="flex flex-col gap-1.5">
      <div className={OUTPUT_BOX}>
        {shown.map((line, i) => (
          <div key={i} className={cn(MONO, "min-w-max text-muted-foreground")}>
            {line || " "}
          </div>
        ))}
      </div>
      {capped ? (
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="xs" onClick={() => setFull(true)}>
            {`Show all ${lines.length} lines`}
          </Button>
          {outputPath ? (
            <Button variant="ghost" size="xs" onClick={() => agentTools.openPath(outputPath)}>
              <ExternalLink />
              Open full output
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function Steps({ steps }) {
  return (
    <ol className="flex flex-col gap-1">
      {steps.map((step, i) => (
        <li key={i} className="flex items-start gap-2 text-[12px] leading-relaxed text-muted-foreground">
          <span className="w-4 shrink-0 tabular-nums text-muted-foreground/60">{i + 1}</span>
          <span className="min-w-0 flex-1">{typeof step === "string" ? step : (step?.title ?? asText(step))}</span>
        </li>
      ))}
    </ol>
  );
}

/** A small fact from metadata - "12 matches", "340 lines". */
function Counts({ items }) {
  const shown = items.filter((item) => item[1] != null);
  if (!shown.length) return null;
  return (
    <div className="flex flex-wrap gap-1.5">
      {shown.map(([label, value]) => (
        <Badge key={label} size="sm" variant="neutral">
          {`${value} ${label}`}
        </Badge>
      ))}
    </div>
  );
}

/**
 * The expanded body, chosen by tool.
 *
 * A generic key/value dump is the fallback, not the plan. Every tool here has
 * one thing the reader came for - the diff, the checklist, the exit code - and
 * burying it in a list of arguments wastes the only moment they were looking.
 */
/** The image a `read` call returned, at a size that fits in a transcript. */
function ImageResult({ path, bytes }) {
  const disk = useLocalImage(path);
  const [open, setOpen] = React.useState(false);
  const name = String(path).split(/[\\/]/).pop();

  if (!disk.url) {
    return (
      <p className="text-[11px] text-muted-foreground">
        {disk.failed ? `${name} could not be read from disk.` : name}
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="overflow-hidden rounded-xl border border-border-subtle transition-[border-color] hover:border-border-strong"
      >
        <img src={disk.url} alt={name} className="max-h-64 w-auto max-w-full object-contain" />
      </button>
      <p className="text-[11px] text-muted-foreground">{name}</p>
      {/* Always mounted, given a file or nothing: the viewer keeps itself on
          the page for its own exit, which it cannot do if closing unmounts it. */}
      <FilePreview
        file={open ? { name, kind: "image", dataUrl: disk.url, path, size: bytes } : null}
        onClose={() => setOpen(false)}
      />
    </div>
  );
}

function Body({ call }) {
  const { name, args = {}, metadata = {}, output, state } = call;
  const key = keyForTool(name);

  if (key === "edit" && metadata.diff) {
    return (
      <div className="flex flex-col gap-2">
        <ToolDiff diff={metadata.diff} />
        <Args args={args} skip={["content", "oldString", "newString", "old_string", "new_string", "diff"]} />
      </div>
    );
  }

  // A file that did not exist before has nothing to diff against, so this used
  // to fall through to the generic argument dump: the path, then the first two
  // hundred characters of the file on one wrapped line behind a "show more".
  // Eleven new components in a row rendered as eleven of those, which is how a
  // card that is supposed to show what happened ends up showing nothing. The
  // file is right here in the arguments - so it is shown as the file it is.
  if (name === "write" && !metadata.diff && typeof args.content === "string") {
    const file = metadata.path ?? args.filePath ?? args.path ?? call.title;
    return (
      <div className="flex flex-col gap-2">
        <p className={cn(MONO, "break-all text-muted-foreground")}>{file}</p>
        <Counts items={[["lines", args.content.split("\n").length]]} />
        <CodeBlock lang={langFor(file)} code={args.content} />
      </div>
    );
  }

  if (name === "todowrite" && Array.isArray(metadata.todos)) {
    return <TaskList todos={metadata.todos} />;
  }

  // A picture the agent looked at is shown to the person too. The agent was
  // handed the image itself, so a card here reading "screenshot.png, 84 KB" is
  // strictly less than what the model got.
  if (name === "read" && metadata.image && metadata.path) {
    return <ImageResult path={metadata.path} bytes={metadata.bytes} />;
  }

  // A plan is the one tool result that is also a question, so it renders as the
  // card that asks it rather than as a wall of arguments.
  if (name === "present_plan" && metadata.plan) {
    return <PlanCard plan={metadata.plan} callId={call.callId} />;
  }

  if (name === "task") {
    const agent = metadata.agent ?? args.subagent_type ?? args.agent ?? "Subagent";
    return (
      <div className="flex flex-col gap-2">
        <p className="text-[11px] text-muted-foreground">{agent}</p>
        {Array.isArray(metadata.steps) && metadata.steps.length ? <Steps steps={metadata.steps} /> : null}
        <Args args={args} skip={["subagent_type", "agent"]} />
        <Output output={output} outputPath={metadata.outputPath} />
      </div>
    );
  }

  if (name === "shell") {
    const exit = metadata.exitCode ?? metadata.exit_code;
    return (
      <div className="flex flex-col gap-2">
        <div className={cn(OUTPUT_BOX, "flex items-start gap-2")}>
          <span aria-hidden="true" className={cn(MONO, "select-none text-muted-foreground")}>
            $
          </span>
          <span className={cn(MONO, "min-w-0 flex-1 text-foreground")}>{args.command ?? call.title}</span>
        </div>
        {exit ? (
          <Badge variant="danger" size="sm">
            {`exit ${exit}`}
          </Badge>
        ) : null}
        <Args args={args} skip={["command"]} />
        <Output output={output} outputPath={metadata.outputPath} />
      </div>
    );
  }

  if (name === "read" || key === "glob" || key === "grep" || name === "ls") {
    const target = args.filePath ?? args.path ?? args.pattern ?? call.title;
    return (
      <div className="flex flex-col gap-2">
        <p className={cn(MONO, "break-all text-muted-foreground")}>{target}</p>
        <Counts
          items={[
            ["matches", metadata.matches ?? metadata.count],
            ["files", metadata.files],
            ["lines", metadata.lines],
          ]}
        />
        <Args args={args} skip={["filePath", "path", "pattern"]} />
        {/* Why it failed, when the tool said nothing. A failed read that shows
            only the path it failed on tells the reader what they already knew
            from the row above it. */}
        {state === "failed" && !asText(output).trim() ? (
          <p className="text-[12px] leading-relaxed text-destructive-ink">
            {metadata.error ?? "The file could not be read."}
          </p>
        ) : null}
        <Output output={output} outputPath={metadata.outputPath} />
      </div>
    );
  }

  // A tool from an MCP server. Nobody here knows what it does, so its result is
  // read for its shape rather than for its name - see McpResult. The arguments
  // stay above it, because for a server tool the call is half the story: which
  // query was actually sent is the first thing anyone checks when the answer
  // looks wrong.
  if (metadata.server && metadata.tool) {
    return (
      <div className="flex flex-col gap-2">
        <Args args={args} />
        {state === "failed" && !asText(output).trim() ? (
          <p className="text-[12px] leading-relaxed text-destructive-ink">
            {metadata.error ?? "The tool did not report why it failed."}
          </p>
        ) : null}
        <McpResult
          output={asText(output)}
          metadata={metadata}
          fallback={<Output output={output} outputPath={metadata.outputPath} />}
        />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <Args args={args} />
      {state === "failed" && !asText(output).trim() ? (
        <p className="text-[12px] leading-relaxed text-destructive-ink">
          {metadata.error ?? "The tool did not report why it failed."}
        </p>
      ) : null}
      <Output output={output} outputPath={metadata.outputPath} />
    </div>
  );
}

/**
 * The demo transcript still speaks the old block shape. Rather than fork the
 * card, translate: the two shapes describe the same event, and the adapter is
 * cheaper than a second component that will drift.
 */
function fromBlock(block) {
  if (!block) return null;
  const output = block.output ?? {};
  return {
    callId: block.id,
    name: block.tool,
    title: block.title,
    args: block.input ?? {},
    state: block.state === "error" ? "failed" : (block.state ?? "done"),
    output: Array.isArray(output.lines) ? output.lines : (output.summary ?? output),
    metadata: Array.isArray(output.diff) ? { diff: output.diff.join("\n") } : {},
    durationMs: block.durationMs,
  };
}

/**
 * The calls that report themselves while they happen and then leave nothing.
 *
 * Remembering something is not an action a reader wants a receipt for. It is
 * the agent thinking, and a transcript that keeps a folded card for every
 * recall reads like a machine filing paperwork rather than someone who knows
 * things. Claude and Codex both show a line while it happens and then let it go,
 * and that is right: the evidence a memory was written is the Memory screen, not
 * a row in the conversation.
 *
 * A failure is the exception, and it stays. Silently failing to save something
 * the user asked to be remembered is exactly the bug this would otherwise hide.
 */
const QUIET = {
  memory_recall: "Reading memory",
  memory_save: "Writing memory",
  memory_forget: "Forgetting",
  // Loading its own tools is the purest plumbing in the app. "Loaded 2 tools"
  // is not news to anybody, and a folded card for it sat in the transcript
  // above the answer forever.
  load_tools: "Getting more tools",
};

/**
 * How long a quiet line stays once it has appeared.
 *
 * Saving a memory locally is a file write and takes about two milliseconds, so
 * the honest rendering - show while running, stop when done - was a line that
 * appeared and vanished inside a single frame. Nobody saw it, which is the same
 * as it not existing while still being code that has to be maintained.
 *
 * So once shown it stays for a beat. Long enough to read, short enough that it
 * is gone before anyone looks for it.
 */
const MIN_VISIBLE_MS = 900;

function QuietCall({ call }) {
  const verb = QUIET[call.name];
  const running = call.state === "running";

  // Seeded from the state at mount, so a finished call scrolled back into view
  // in an old conversation shows nothing at all rather than flashing.
  const [visible, setVisible] = React.useState(running);
  const startedAt = React.useRef(running ? Date.now() : 0);

  React.useEffect(() => {
    if (running) {
      if (!startedAt.current) startedAt.current = Date.now();
      setVisible(true);
      return undefined;
    }
    if (!visible) return undefined;

    const remaining = Math.max(0, MIN_VISIBLE_MS - (Date.now() - startedAt.current));
    const timer = setTimeout(() => setVisible(false), remaining);
    return () => clearTimeout(timer);
  }, [running, visible]);

  // Held for one more beat on the way out, so the line fades rather than
  // ceasing to exist - the whole of this is the movement, so its end should be
  // one too.
  const presence = usePresence(visible);

  if (call.state === "failed") {
    return (
      <div
        data-tool-call={call.name}
        data-tool-state="failed"
        className="my-1 flex items-center gap-2 px-1 text-[13px] text-destructive-ink"
      >
        <TriangleAlert aria-hidden="true" className="size-3.5 shrink-0" />
        <span className="min-w-0 truncate">{asText(call.output) || `${verb} failed.`}</span>
      </div>
    );
  }

  // Done is nothing at all. Not a collapsed card, not an empty row - the line
  // was the feedback and the feedback is over.
  if (!presence.mounted) return null;

  return (
    <div
      data-tool-call={call.name}
      data-tool-state={running ? "running" : "settling"}
      data-state={presence.state}
      className="my-1 flex items-center gap-2 px-1 animate-fade-in"
    >
      <span className="text-shimmer text-[13px]">{verb}...</span>
      <span className="sr-only" role="status">
        {verb}
      </span>
    </div>
  );
}

/**
 * One call inside a folded run.
 *
 * Deliberately not a `ToolCallCard`: a card inside a card is two fills, two
 * radii and two rows of chrome for one call. This is the same row stripped to
 * what changes between siblings - which target, how it went - because the tool
 * and the verb are already said once above.
 */
function RunRow({ call }) {
  const image = Boolean(call.metadata?.image);
  const failed = call.state === "failed";
  // A failure is the row worth reading, so it is the row that opens itself.
  const [open, setOpen] = React.useState(failed || image);
  const read = call.name === GROUPED_TOOL;
  // The tool's own glyph for anything that is not a file. Repeating the
  // group's icon down the rows is duller than inventing a second meaning for a
  // shape the reader already knows.
  const RowIcon = image ? getIcon("Image") : read ? getIcon("FileText") : iconFor(call.name);

  return (
    <div className="min-w-0">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex h-8 w-full items-center gap-2 rounded-xl px-2 text-left outline-none",
          "transition-colors duration-150 ease-out hover:fill-nav focus-visible:fill-nav",
        )}
      >
        <RowIcon
          aria-hidden="true"
          className={cn("size-3.5 shrink-0", failed ? "text-destructive-ink" : "text-muted-foreground")}
        />
        <Title text={read ? fileName(call) : (call.title ?? call.name ?? "")} />
        <StateMark state={call.state} durationMs={call.durationMs} />
        <ChevronRight aria-hidden="true" className={cn(CHEVRON, open && "rotate-90")} />
      </button>
      <Collapse open={open}>
        <div className={cn("px-2 pb-2 pt-0.5", CARD_BODY)}>
          <Body call={call} />
        </div>
      </Collapse>
    </div>
  );
}

/**
 * A run of calls to one tool, as one row.
 *
 * The header is the same shape as every other tool row on purpose - icon,
 * verb, what it touched, how it went - so a folded run scans as one action
 * rather than as a new kind of object. What is behind it is the calls, each
 * still openable, in the order they were made.
 */
export function ToolRun({ calls, className }) {
  const state = runState(calls);
  const failed = state === "failed";
  const [open, setOpen] = React.useState(failed);

  // Same rule as a single card: a run that fails after the fact opens itself,
  // and only on the transition, so a reader who folded it keeps it folded.
  const wasFailed = React.useRef(failed);
  React.useEffect(() => {
    if (failed && !wasFailed.current) setOpen(true);
    wasFailed.current = failed;
  }, [failed]);

  const name = calls[0]?.name ?? GROUPED_TOOL;
  const ToolIcon = iconFor(name);
  const label = toolByKey(keyForTool(name))?.label ?? name;

  // A run that mounts while its first call is still going is one appearing
  // mid-reply; a finished one is history being scrolled to. See arrival.js.
  const arrival = useArrival(calls[0]?.callId, { live: state === "running" });

  return (
    <div
      data-tool-call={name}
      data-tool-group={calls.length}
      data-tool-state={state}
      onAnimationEnd={arrival.onAnimationEnd}
      className={cn(
        "my-2 rounded-2xl card-surface-subtle dark:card-surface-subtle",
        arrival.arriving && "animate-fade-in",
        className,
      )}
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex h-9 w-full items-center gap-2 rounded-2xl px-3 text-left outline-none",
          "transition-colors duration-150 ease-out hover:fill-nav",
          "focus-visible:fill-nav",
        )}
      >
        <ToolIcon
          aria-hidden="true"
          className={cn("size-3.5 shrink-0", failed ? "text-destructive-ink" : "text-muted-foreground")}
        />
        <span className="shrink-0 text-[13px] text-muted-foreground">{label}</span>
        <Title text={runTitle(calls)} />
        <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/70">
          {runCount(calls)}
        </span>
        <StateMark state={state} durationMs={runDuration(calls)} />
        <ChevronRight aria-hidden="true" className={cn(CHEVRON, open && "rotate-90")} />
      </button>

      <Collapse open={open} innerClassName="flex flex-col px-1 pb-1.5 pt-0.5">
        {calls.map((call, index) => (
          <RunRow key={call.callId ?? index} call={call} />
        ))}
      </Collapse>
    </div>
  );
}

/**
 * One tool call.
 *
 * Memoised for the same reason `MessageBubble` is, and it matters more here: a
 * tool card renders a diff, a syntax-highlighted file, or a whole MCP result,
 * and a reply that used forty tools is forty of those. Its props are a part or
 * a block object and a class name, all of which are stable while the call is
 * not changing, so a streaming reply repaints only the card it is writing.
 */
function ToolCallCardInner({ part, block, className }) {
  const call = part ?? fromBlock(block);
  // A picture opens itself. The row for an image read says "screenshot.png,
  // 84 KB", which is the one thing about a picture that does not matter, and
  // the agent was handed the picture itself - so the reader gets it too,
  // without having to guess that there is something behind the chevron.
  const [open, setOpen] = React.useState(
    call?.state === "failed" || Boolean(call?.metadata?.image),
  );

  // A call that fails after the fact opens itself. Only on the transition, so
  // a reader who collapsed a failure keeps it collapsed.
  const failed = call?.state === "failed";
  const wasFailed = React.useRef(failed);
  React.useEffect(() => {
    if (failed && !wasFailed.current) setOpen(true);
    wasFailed.current = failed;
  }, [failed]);

  // A card that mounts while its call is running is appearing in the middle
  // of a reply and fades in; one that mounts finished is history scrolling
  // into view and is simply there. Decided once, at mount - see arrival.js -
  // so the tokens streaming through the card afterwards do not restart it.
  const arrival = useArrival(call?.callId, { live: call?.state === "running" });

  if (!call) return null;
  if (QUIET[call.name]) return <QuietCall call={call} />;
  // Handing files over is not a step to be folded away with the rest of the
  // work - it is the work, arriving. It draws itself, open, and only once it
  // has something to draw: while it is still running there is nothing to show
  // and a placeholder row would be a card that flickers into existence.
  if (call.name === "present" && call.metadata?.presented?.length) {
    return (
      <PresentedFiles
        files={call.metadata.presented}
        note={call.metadata.note}
        onAnimationEnd={arrival.onAnimationEnd}
        className={cn(arrival.arriving && "animate-fade-in", className)}
      />
    );
  }

  const { name, title, state, metadata = {}, durationMs } = call;
  const ToolIcon = iconFor(name);
  const label = TOOL_LABEL[name] ?? toolByKey(keyForTool(name))?.label ?? name;
  const streaming = state === "running" ? lastLine(metadata.output) : "";

  return (
    <div
      data-tool-call={name}
      data-tool-state={state}
      onAnimationEnd={arrival.onAnimationEnd}
      className={cn(
        "my-2 rounded-2xl card-surface-subtle dark:card-surface-subtle",
        arrival.arriving && "animate-fade-in",
        className,
      )}
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex h-9 w-full items-center gap-2 rounded-2xl px-3 text-left outline-none",
          "transition-colors duration-150 ease-out hover:fill-nav",
          "focus-visible:fill-nav"
        )}
      >
        <ToolIcon
          aria-hidden="true"
          className={cn("size-3.5 shrink-0", failed ? "text-destructive-ink" : "text-muted-foreground")}
        />
        <span className="shrink-0 text-[13px] text-muted-foreground">{label}</span>
        <Title text={title ?? name} fromLeft={PATHISH.has(name)} />
        {/* The last line of a running command. A build scrolling past is what
            makes a two-minute command feel alive rather than hung. */}
        {streaming ? (
          <span className="hidden min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground/70 sm:block">
            {streaming}
          </span>
        ) : null}
        <StateMark state={state} durationMs={durationMs} />
        <ChevronRight aria-hidden="true" className={cn(CHEVRON, open && "rotate-90")} />
      </button>

      {/* The scrolling box stays inside the fold rather than being the fold:
          the fold clips while it moves, and a box that scrolls in one axis and
          clips in the other shows a scrollbar for the length of the motion. */}
      <Collapse open={open}>
        <div className={cn("px-3 pb-3 pt-0.5", CARD_BODY)}>
          <Body call={call} />
        </div>
      </Collapse>
    </div>
  );
}

export const ToolCallCard = React.memo(ToolCallCardInner);
