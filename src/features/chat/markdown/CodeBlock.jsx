import * as React from "react";
import { Check, Copy, Eye, Play, Rows3, Square } from "@/components/icons";
import { IconButton } from "@/components/ui/icon-button";
import { cn } from "@/lib/utils";
import { copyText } from "@/lib/clipboard";
import { languageOf, tokenize } from "./highlight.js";
import { RunFolder } from "./run-folder.js";

const TOKEN_CLASS = {
  comment: "tok-comment",
  string: "tok-string",
  number: "tok-number",
  keyword: "tok-keyword",
  function: "tok-function",
  type: "tok-type",
  property: "tok-property",
  operator: "tok-operator",
  punctuation: "tok-punctuation",
  tag: "tok-tag",
  attribute: "tok-attribute",
  meta: "tok-meta",
  added: "tok-added",
  removed: "tok-removed",
};

/** Fences whose contents are a command line rather than a program. */
const SHELLS = /^(sh|bash|zsh|shell|console|shellsession|powershell|pwsh|ps1|bat|cmd|dos)$/i;

/**
 * Whether a block in this language can be run on this machine.
 *
 * A shell fence always can. Anything else depends on what is installed, which
 * only the main process knows - Python on one machine and not the next - so it
 * is asked once per language and remembered. A button that would always fail
 * is worse than no button, which is why this is a question rather than a list.
 */
const asked = new Map();

function useRunnable(tag) {
  const shell = SHELLS.test(tag);
  const [ready, setReady] = React.useState(() => shell || asked.get(tag) === true);

  React.useEffect(() => {
    if (shell || !tag || !window.electronAPI?.canRun) return undefined;
    if (asked.has(tag)) {
      setReady(asked.get(tag));
      return undefined;
    }
    let live = true;
    Promise.resolve(window.electronAPI.canRun(tag))
      .then((answer) => {
        asked.set(tag, Boolean(answer));
        if (live) setReady(Boolean(answer));
      })
      .catch(() => live && setReady(false));
    return () => {
      live = false;
    };
  }, [tag, shell]);

  return (shell || ready) && Boolean(window.electronAPI?.runSnippet);
}

/** Fences that are a page, and can therefore be shown as one. */
const MARKUP = /^(html|svg|xml)$/i;

/** Longer than this and wrapping actually changes what is on screen. */
const LONG_LINE = 88;

/**
 * The code, in colour.
 *
 * Memoised on the text rather than rendered inline, because a streaming reply
 * re-renders its last block on every token and the last block is often the
 * code one. Re-scanning two hundred lines sixty times a second is the kind of
 * cost that only shows up on someone else's machine.
 */
const Highlighted = React.memo(function Highlighted({ code, lang }) {
  const spans = React.useMemo(() => tokenize(code, lang), [code, lang]);
  return spans.map((span, index) =>
    span.kind === "plain" ? (
      <React.Fragment key={index}>{span.text}</React.Fragment>
    ) : (
      <span key={index} className={TOKEN_CLASS[span.kind]}>
        {span.text}
      </span>
    )
  );
});

/**
 * A page a model wrote, shown as a page.
 *
 * `sandbox="allow-same-origin"` and deliberately not `allow-scripts`: with no
 * scripts nothing inside can act, and being same-origin is what lets this
 * measure the content and give the frame the height it actually needs instead
 * of a fixed box with a scrollbar in it. The two together are the whole reason
 * this is safe - `allow-scripts` beside `allow-same-origin` would undo the
 * sandbox entirely, and neither is worth a running script from a model.
 */
function Markup({ code }) {
  const frame = React.useRef(null);
  const [height, setHeight] = React.useState(220);

  const measure = React.useCallback(() => {
    const document_ = frame.current?.contentDocument;
    const body = document_?.body;
    if (!body) return;
    // The bottom of the body, not `scrollHeight`. A body never shorter than
    // the frame it is in makes `scrollHeight` measure the frame - so the box
    // kept whatever height it started with, which is the empty white space
    // under a two-line card.
    const wanted = body.getBoundingClientRect().bottom + 12;
    if (wanted > 0) setHeight(Math.min(Math.max(Math.ceil(wanted), 60), 720));
  }, []);

  // Once on load, and again a beat later: web fonts and images finish after
  // the load event and a frame measured too early is a frame with its last
  // paragraph cut off.
  React.useEffect(() => {
    const timer = window.setTimeout(measure, 250);
    return () => window.clearTimeout(timer);
  }, [measure, code]);

  return (
    <iframe
      ref={frame}
      title="Rendered markup"
      sandbox="allow-same-origin"
      onLoad={measure}
      srcDoc={`<!doctype html><meta charset="utf-8"><style>body{margin:12px;font-family:system-ui,sans-serif}</style>${code}`}
      style={{ height }}
      className="w-full border-0 bg-white"
    />
  );
}

/** What came back from a run, under the code that produced it. */
function Output({ result, running }) {
  if (running) {
    return (
      <div className="border-t border-border-subtle px-3 py-2 text-[11px] text-muted-foreground">
        Running...
      </div>
    );
  }
  if (!result) return null;

  const failed = !result.ok;
  return (
    <div className="border-t border-border-subtle px-3 py-2">
      <div className="mb-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-muted-foreground">
        <span className={cn(failed && "text-destructive-ink")}>
          {result.error
            ? result.error
            : result.timedOut
              ? "Stopped after two minutes"
              : result.ok
                ? "Finished"
                : `Exit code ${result.code}`}
        </span>
        {result.cwd ? <span className="truncate opacity-70">in {result.cwd}</span> : null}
      </div>
      {result.output?.trim() ? (
        <pre className="no-scrollbar max-h-72 overflow-auto whitespace-pre-wrap wrap-break-word font-mono text-[12px] leading-relaxed text-foreground">
          {result.truncated ? "... (earlier output dropped)\n" : ""}
          {result.output.trimEnd()}
        </pre>
      ) : null}
    </div>
  );
}

/**
 * A fenced code block.
 *
 * The block owns its own horizontal scroll: `min-w-0` on the shell plus
 * `overflow-x-auto` on the `pre` means a 400-character line scrolls inside the
 * card instead of pushing the message column - and the whole transcript -
 * sideways.
 *
 * Every control here appears only when it would do something. Wrapping is
 * offered when a line is long enough for it to change what is on screen, a
 * page is offered a preview, and a command is offered a Run button - and a
 * block of TypeScript gets none of the three, because a button that does
 * nothing when pressed is worse than an absent one.
 */
export function CodeBlock({ lang, code, closed = true }) {
  // The fence tag as written when it says something, so `tsx` stays `tsx`
  // rather than being renamed to the grammar it happens to share with `ts`.
  const label = lang || (languageOf(lang) ? languageOf(lang) : "text");
  const tag = String(lang ?? "").trim();
  // Hooks before any early return, and the runnable check is one: it asks the
  // main process what this machine has.
  const [copied, setCopied] = React.useState(false);
  const [wrap, setWrap] = React.useState(false);
  const [rendered, setRendered] = React.useState(false);
  const [running, setRunning] = React.useState(false);
  const [result, setResult] = React.useState(null);
  const timer = React.useRef(0);
  const folder = React.useContext(RunFolder);

  const wrappable = React.useMemo(
    () => code.split("\n").some((line) => line.length > LONG_LINE),
    [code]
  );
  const previewable = MARKUP.test(tag);
  const runnable = useRunnable(tag);

  React.useEffect(() => () => window.clearTimeout(timer.current), []);

  const copy = async () => {
    if (!(await copyText(code))) return;
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), 1400);
  };

  const run = async () => {
    if (running) return;
    setRunning(true);
    setResult(null);
    try {
      const answer = await window.electronAPI.runSnippet({ command: code, lang: tag, cwd: folder });
      setResult(answer ?? { ok: false, error: "Nothing came back." });
    } catch (error) {
      setResult({ ok: false, error: error?.message ?? "That could not be run." });
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="my-3 min-w-0 overflow-hidden rounded-2xl card-surface-subtle">
      <div className="flex h-8 items-center justify-between pl-3 pr-1.5">
        <span className="truncate text-[11px] text-muted-foreground">{label}</span>
        <div className="flex items-center gap-0.5">
          {runnable ? (
            <IconButton
              size="sm"
              label={running ? "Running" : "Run this here"}
              active={running}
              onClick={run}
            >
              {running ? <Square /> : <Play />}
            </IconButton>
          ) : null}
          {previewable ? (
            <IconButton
              size="sm"
              label={rendered ? "Show the markup" : "Render it"}
              active={rendered}
              onClick={() => setRendered((value) => !value)}
            >
              <Eye />
            </IconButton>
          ) : null}
          {wrappable ? (
            <IconButton
              size="sm"
              label={wrap ? "Do not wrap lines" : "Wrap long lines"}
              active={wrap}
              onClick={() => setWrap((value) => !value)}
            >
              <Rows3 />
            </IconButton>
          ) : null}
          <IconButton size="sm" label="Copy code" onClick={copy}>
            {copied ? <Check className="text-success-ink" /> : <Copy />}
          </IconButton>
        </div>
      </div>
      {rendered ? <Markup code={code} /> : null}
      <pre
        hidden={rendered}
        className={cn("no-scrollbar overflow-x-auto px-3 pb-3 pt-0.5", wrap && "overflow-x-hidden")}
      >
        <code
          className={cn(
            "font-mono text-[12.5px] leading-relaxed text-foreground",
            wrap ? "block whitespace-pre-wrap wrap-break-word" : "inline-block"
          )}
        >
          {/* Plain until the fence closes. Highlighting is a full scan of the
              block, the block grows by a few characters per token, and the
              memo is keyed on the text - so a model writing a 400-line file
              re-highlighted all of it several thousand times on the way, which
              measured at three seconds of main-thread work and a hundred
              megabytes of garbage for one reply, rising with the square of the
              file. Nobody can read colour arriving a token at a time anyway.
              The moment the fence closes this swaps to the real thing. */}
          {closed ? <Highlighted code={code} lang={lang} /> : code}
        </code>
      </pre>
      <Output result={result} running={running} />
    </div>
  );
}
