import * as React from "react";
import { Eraser, Square, TerminalSquare } from "@/components/icons";
import { cn } from "@/lib/utils";
import { IconButton } from "@/components/ui/icon-button";
import { EmptyState } from "@/components/ui/empty-state";
import { runCommand, workdirOf } from "@/lib/computers";

/**
 * A terminal that reaches the machine.
 *
 * What was here before answered four commands from a lookup table and told you
 * so in the output of everything else. This one spawns a real process: in a
 * container for Docker, in the cloud for Daytona, on this computer for local.
 *
 * Not a PTY, and the difference is worth naming rather than hiding. Each line
 * is one command, run and collected - so `cd` does not persist between them,
 * and anything that wants a terminal to talk to (vim, top, a password prompt)
 * will hang until it times out rather than drawing. A real PTY needs a
 * pseudo-terminal on the other side and a terminal emulator on this one, which
 * is a dependency and a protocol; one command at a time covers what people
 * actually type into a pane like this, and the working directory is carried
 * explicitly so the common case still behaves.
 */

const STREAM_INK = {
  in: "text-foreground",
  out: "text-muted-foreground",
  err: "text-destructive-ink",
  note: "text-muted-foreground/60 italic",
};

export function TerminalPane({ computer }) {
  /** The working directory this machine's pane is in, kept for the session. */
  const DEFAULT_CWD = workdirOf(computer);
  const [lines, setLines] = React.useState([]);
  const [draft, setDraft] = React.useState("");
  const [cwd, setCwd] = React.useState(DEFAULT_CWD);
  const [running, setRunning] = React.useState(null);
  const [history, setHistory] = React.useState([]);
  const [historyAt, setHistoryAt] = React.useState(-1);
  const scrollRef = React.useRef(null);

  // A new machine is a new session. Keeping the old machine's output would be
  // a transcript attributed to the wrong computer.
  React.useEffect(() => {
    setLines([]);
    setCwd(DEFAULT_CWD);
    setRunning(null);
  }, [computer.id]);

  React.useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines]);

  const append = React.useCallback((line) => {
    setLines((prev) => [...prev, line]);
  }, []);

  if (computer.status !== "running") {
    return (
      <EmptyState
        className="min-h-0 flex-1"
        icon={TerminalSquare}
        title={`${computer.name} is ${computer.status}`}
        description="Start the machine to run something on it."
      />
    );
  }

  async function submit(event) {
    event.preventDefault();
    const text = draft.trim();
    if (!text || running) return;

    setDraft("");
    setHistory((prev) => (prev[prev.length - 1] === text ? prev : [...prev, text]));
    setHistoryAt(-1);

    if (text === "clear") {
      setLines([]);
      return;
    }

    // `cd` is handled here rather than sent, because each command is its own
    // process and a `cd` that ran in the machine would be forgotten the moment
    // it exited. Carrying the directory is what makes the pane feel like a
    // session without pretending to be one.
    if (/^cd(\s|$)/.test(text)) {
      const target = text.slice(2).trim() || DEFAULT_CWD;
      append({ stream: "in", text });
      const resolved = target.startsWith("/")
        ? target
        : `${cwd.replace(/\/+$/, "")}/${target}`.replace(/\/\.$/, "");
      setCwd(resolved);
      append({ stream: "note", text: `Working directory is now ${resolved}` });
      return;
    }

    append({ stream: "in", text });

    // Whether anything arrived while the command was running. No provider
    // streams today - every one of them answers in a single piece at the end -
    // so this is almost always false, and the pane has to print the answer
    // itself. It used to assume the opposite, which is why a command that
    // worked printed nothing at all and a command that failed printed only its
    // exit code.
    let streamed = false;

    const run = runCommand(
      computer.id,
      { command: text, cwd, timeoutMs: 300_000 },
      {
        onOutput: (chunk) => {
          streamed = true;
          append({ stream: chunk.stream === "stderr" ? "err" : "out", text: chunk.text });
        },
      }
    );
    setRunning(run);

    try {
      const result = await run.promise;
      if (!streamed) {
        if (result.stdout) append({ stream: "out", text: trimEnd(result.stdout) });
        if (result.stderr) append({ stream: "err", text: trimEnd(result.stderr) });
      }
      // What the output itself cannot say: whether it worked.
      if (result.timedOut) append({ stream: "err", text: "Timed out." });
      else if (result.code !== 0) append({ stream: "note", text: `Exit code ${result.code}` });
      else if (!streamed && !result.stdout && !result.stderr) {
        // Silence and a zero is a success, but an empty pane reads like a
        // command that never ran.
        append({ stream: "note", text: "No output." });
      }
    } catch (error) {
      append({ stream: "err", text: error?.message ?? String(error) });
    } finally {
      setRunning(null);
    }
  }

  /** Drop the trailing newline every shell adds; keep the blank lines inside. */
  function trimEnd(text) {
    return String(text).replace(/\s+$/, "");
  }

  function onKeyDown(event) {
    // Up and down through what was typed, which is the one thing every shell
    // has and the absence of which makes a pane like this exhausting.
    if (event.key === "ArrowUp") {
      event.preventDefault();
      const next = historyAt === -1 ? history.length - 1 : Math.max(0, historyAt - 1);
      if (history[next] !== undefined) {
        setHistoryAt(next);
        setDraft(history[next]);
      }
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (historyAt === -1) return;
      const next = historyAt + 1;
      if (next >= history.length) {
        setHistoryAt(-1);
        setDraft("");
      } else {
        setHistoryAt(next);
        setDraft(history[next]);
      }
    }
  }

  const prompt = `agent@${String(computer.name).toLowerCase().replace(/\s+/g, "-")}:${cwd}`;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex shrink-0 items-center justify-between gap-2">
        <p className="truncate text-[11px] text-muted-foreground">
          One command per line on {computer.name}. Not a full terminal: interactive programs
          will not draw.
        </p>
        <div className="flex shrink-0 items-center gap-1">
          {running ? (
            <IconButton size="lg" label="Stop the command" onClick={() => running.cancel()}>
              <Square />
            </IconButton>
          ) : null}
          <IconButton size="lg" label="Clear" onClick={() => setLines([])}>
            <Eraser />
          </IconButton>
        </div>
      </div>

      <div className="flex min-h-44 flex-1 flex-col rounded-2xl bg-card-darker p-3 font-mono text-[12.5px] leading-relaxed">
        <p className="shrink-0 text-muted-foreground/70">{prompt}</p>

        <div
          ref={scrollRef}
          className="no-scrollbar mt-1 min-h-0 flex-1 overflow-y-auto overscroll-contain"
        >
          {lines.map((line, i) => (
            <p
              key={i}
              className={cn(
                "whitespace-pre-wrap break-words",
                STREAM_INK[line.stream] ?? STREAM_INK.out
              )}
            >
              {line.stream === "in" ? <span className="text-muted-foreground/70">$ </span> : null}
              {line.text}
            </p>
          ))}
          {lines.length === 0 ? (
            <p className="text-muted-foreground/60">
              Nothing has run in this pane yet. Try `ls -la` or `uname -a`.
            </p>
          ) : null}
          {running ? <p className="text-muted-foreground/60">Running…</p> : null}
        </div>

        <form onSubmit={submit} className="mt-2 flex shrink-0 items-center gap-2">
          <span aria-hidden="true" className="text-muted-foreground/70">
            $
          </span>
          <input
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onKeyDown}
            disabled={Boolean(running)}
            aria-label={`Command for ${computer.name}`}
            placeholder={running ? "Waiting for the last command…" : "Type a command"}
            spellCheck={false}
            autoComplete="off"
            className={cn(
              "min-w-0 flex-1 bg-transparent font-mono text-[12.5px] text-foreground outline-none",
              "placeholder:text-muted-foreground/50",
              "selection:bg-foreground selection:text-background",
              "disabled:opacity-50"
            )}
          />
        </form>
      </div>
    </div>
  );
}
