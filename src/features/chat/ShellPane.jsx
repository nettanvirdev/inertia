import * as React from "react";
import { X } from "@/components/icons";
import { IconButton } from "@/components/ui/icon-button";
import { cn } from "@/lib/utils";
import { isTerminalAvailable, terminal } from "@/lib/terminal";
import { xtermTheme } from "@/features/chat/xterm-theme.js";
import {
  activeOf,
  addTab,
  adoptTab,
  closeTab,
  kindOfTab,
  labelFor,
} from "@/features/chat/pane-tabs.js";
import { clearReveal, onRevealPane } from "@/features/chat/pane-reveal";
import { PaneTabStrip } from "@/features/chat/PaneTabStrip";
import { copyText, readClipboardText } from "@/lib/clipboard";
import "@xterm/xterm/css/xterm.css";

/**
 * Terminals, beside the conversation.
 *
 * The pane has two shapes because the backend has two ways to run a shell -
 * see the `inertia-terminal` crate. With a pty it is a real terminal: xterm.js,
 * every keystroke going straight through, `vim` and `htop` drawing, Ctrl+C
 * reaching the program. Without one it is a log and a command box, which is
 * what a piped shell honestly is.
 *
 * ── Why xterm.js and not a <pre> ──────────────────────────────────────────
 * A pty does not emit text. It emits instructions for painting a grid, and a
 * `<pre>` full of those is what `npm install` looks like without an emulator.
 *
 * ── Why the shell outlives the tab it is shown in ─────────────────────────
 * The shell belongs to the conversation. Switching to another dock tab must
 * not kill a running build, and coming back should show what happened while
 * you were gone - which is what the backend's screen is for. Closing a
 * TERMINAL tab is different: that is a person saying they are done with that
 * shell, so it is closed for real.
 */

/** How many commands back the fallback's up arrow reaches. */
const HISTORY = 200;

export function ShellPane({ paneId, cwd, onClose, className }) {
  const available = isTerminalAvailable();
  const [pty, setPty] = React.useState(null);

  const [tabs, setTabs] = React.useState(() => addTab([], { threadId: paneId, kind: "sh" }).tabs);
  const [active, setActive] = React.useState(() => `chat:${paneId}:sh:1`);
  const current = activeOf(tabs, active);

  React.useEffect(() => {
    if (!available) return undefined;
    let alive = true;
    terminal
      .capability()
      .then((found) => alive && setPty(Boolean(found?.pty)))
      .catch(() => alive && setPty(false));
    return () => {
      alive = false;
    };
  }, [available]);

  const openTab = () => {
    const next = addTab(tabs, { threadId: paneId, kind: "sh" });
    setTabs(next.tabs);
    setActive(next.active);
  };

  const dropTab = (id) => {
    const next = closeTab(tabs, current, id);
    // A closed terminal tab is a shell the person is finished with, unlike a
    // dock tab being switched away from.
    terminal.close(id).catch(() => {});
    if (!next.tabs.length) {
      onClose?.();
      return;
    }
    setTabs(next.tabs);
    setActive(next.active);
  };

  const rename = React.useCallback((id, label) => {
    setTabs((prev) =>
      prev.map((tab) => (tab.id === id && tab.label !== label ? { ...tab, label } : tab))
    );
  }, []);

  // `terminal_read` brought a tab forward. Adopting an id this pane does not
  // have is what makes a shell the agent started - or one left open from a
  // previous session - reachable rather than invisible.
  React.useEffect(
    () =>
      onRevealPane(({ dock, tabId, threadId }) => {
        if (dock !== "shell" || threadId !== paneId) return;
        setTabs((prev) => adoptTab(prev, tabId, "sh"));
        setActive(tabId);
        clearReveal("shell", paneId, tabId);
      }),
    [paneId]
  );

  if (!available) {
    return (
      <div className={cn("flex min-h-0 flex-1 items-center justify-center p-6", className)}>
        <p className="text-center text-[12px] leading-relaxed text-muted-foreground">
          The terminal needs the desktop app. This is the web preview, which has no shell to lend.
        </p>
      </div>
    );
  }

  // Nothing is drawn until the answer is known. Guessing and swapping would
  // start a shell, tear down an emulator and start another one, all in the
  // first half second the pane is open.
  if (pty === null) return <div className={cn("min-h-0 flex-1", className)} />;

  return (
    <div className={cn("flex min-h-0 flex-1 flex-col", className)}>
      <PaneTabStrip
        tabs={tabs}
        active={current}
        onSelect={setActive}
        onClose={dropTab}
        onAdd={openTab}
        addLabel="New terminal"
      />
      {tabs.map((tab) => (
        // A mirrored process is always drawn by the emulator. It is a
        // recording of something that had a terminal, so it carries colour and
        // progress bars that only a grid can render - and the piped fallback's
        // command box would be a place to type at a process that has no stdin
        // for us.
        <TabFor
          key={tab.id}
          pty={pty}
          tab={tab}
          cwd={cwd}
          hidden={tab.id !== current}
          onRename={rename}
          onClose={() => dropTab(tab.id)}
        />
      ))}
    </div>
  );
}

/** Which of the two views this tab wants. */
function TabFor({ pty, ...props }) {
  if (kindOfTab(props.tab.id) === "job") return <PtyTab {...props} />;
  return pty ? <PtyTab {...props} /> : <PipeTab {...props} />;
}

/* ── the real terminal ─────────────────────────────────────────────────────── */

function PtyTab({ tab, cwd, hidden, onRename, onClose }) {
  // A tab showing something the app is running, rather than a shell. Nothing
  // to type at, and a stop button that is worth having.
  const job = kindOfTab(tab.id) === "job";
  // Whether the thing being mirrored is still going. A tab opened onto a
  // process that finished before this window did must start at false. Named
  // apart from the effect's own `alive`, which is about this component.
  const [running, setRunning] = React.useState(false);
  const hostRef = React.useRef(null);
  const fitRef = React.useRef(null);
  const [here, setHere] = React.useState("");

  React.useEffect(() => {
    let alive = true;
    let term;
    let offEvent = null;
    let observer = null;
    let themeWatch = null;

    // Imported here rather than at the top of the module so the emulator - a
    // canvas renderer, and not small - is not in the bundle that has to parse
    // before the first conversation paints.
    (async () => {
      const [{ Terminal }, { FitAddon }] = await Promise.all([
        import("@xterm/xterm"),
        import("@xterm/addon-fit"),
      ]);
      if (!alive || !hostRef.current) return;

      term = new Terminal({
        allowProposedApi: true,
        cursorBlink: true,
        fontFamily:
          'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace',
        fontSize: 12,
        lineHeight: 1.25,
        scrollback: 5000,
        // The person selects with the mouse; the shell does not get to.
        rightClickSelectsWord: false,
        theme: xtermTheme(hostRef.current),
      });
      const fit = new FitAddon();
      term.loadAddon(fit);
      term.open(hostRef.current);
      fitRef.current = fit;

      /**
       * Copy and paste, which xterm does not do on its own.
       *
       * Ctrl+C is the hard one, because in a terminal it means interrupt and
       * everywhere else it means copy - and both are right. The rule every
       * terminal has settled on is the one used here: with a selection it
       * copies, without one it interrupts. Ctrl+Shift+C and Ctrl+Shift+V
       * always mean copy and paste, for the muscle memory that expects them.
       *
       * Returning false is what stops the key ALSO being sent to the shell,
       * which is the difference between pasting and pasting followed by a
       * stray ^V.
       */
      const copySelection = () => {
        const text = term.getSelection();
        if (text) void copyText(text);
        return Boolean(text);
      };
      const paste = () => {
        void readClipboardText().then((text) => {
          if (text) terminal.write(tab.id, text);
        });
      };

      term.attachCustomKeyEventHandler((event) => {
        if (event.type !== "keydown") return true;
        const mod = event.ctrlKey || event.metaKey;
        if (!mod) return true;

        if (event.shiftKey && event.code === "KeyC") {
          copySelection();
          return false;
        }
        if (event.shiftKey && event.code === "KeyV") {
          paste();
          return false;
        }
        // Bare Ctrl+C copies only when there is something to copy; otherwise
        // it falls through and becomes the interrupt it is in a terminal.
        if (!event.shiftKey && event.code === "KeyC" && term.hasSelection()) {
          copySelection();
          return false;
        }
        // Bare Ctrl+V has no meaning to a shell, so it is safe to take.
        if (!event.shiftKey && event.code === "KeyV") {
          paste();
          return false;
        }
        return true;
      });

      // Right click: copy a selection, paste when there is none. What a
      // terminal on Windows has always done.
      hostRef.current.addEventListener("contextmenu", (event) => {
        event.preventDefault();
        if (!copySelection()) paste();
      });

      /**
       * The shell is not started until the pane has a real width.
       *
       * A terminal opened at a default size and resized a frame later has
       * already had its first prompt printed at the wrong width, and ConPTY
       * re-wraps what is on screen - which left a stray fragment of the
       * previous prompt on the first line. Waiting for a sane measurement
       * costs a frame or two and means the shell is told the truth from its
       * first byte.
       */
      const sized = async () => {
        for (let attempt = 0; attempt < 40; attempt += 1) {
          try {
            fit.fit();
          } catch {
            // No layout yet.
          }
          if (term.cols >= 20 && term.rows >= 4) return true;
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
        return false;
      };
      await sized();
      if (!alive) return;

      const state = await terminal
        .open(tab.id, { cwd: cwd || undefined, cols: term.cols, rows: term.rows })
        .catch((error) => {
          term.write(`\r\nThe shell could not start.\r\n${error?.message ?? error}\r\n`);
          return null;
        });
      if (!alive) return;
      if (state) {
        // Everything it printed while this tab was closed, or before it
        // existed - replayed through the emulator, so a build that finished
        // five minutes ago looks exactly as it did.
        if (state.scrollback) term.write(state.scrollback);
        setHere(job ? state.title || state.cwd : (state.cwd ?? ""));
        setRunning(Boolean(state.busy));
        onRename(
          tab.id,
          labelFor({
            kind: job ? "job" : "sh",
            serial: tab.serial,
            cwd: state.cwd,
            title: state.title,
          })
        );
      }

      offEvent = terminal.onEvent((event) => {
        if (event?.id !== tab.id) return;
        if (event.type === "data") term.write(event.text);
        else if (event.type === "cwd") {
          setHere(event.cwd ?? "");
          onRename(tab.id, labelFor({ kind: "sh", serial: tab.serial, cwd: event.cwd }));
        } else if (event.type === "exit") term.write("\r\n");
      });

      // A mirror has no stdin of ours. Main ignores writes to one anyway; not
      // sending them keeps a stray keystroke from looking like it did nothing.
      if (!job) term.onData((data) => terminal.write(tab.id, data));
      term.onResize(({ cols, rows }) => terminal.resize(tab.id, cols, rows));

      observer = new ResizeObserver(() => {
        try {
          fit.fit();
        } catch {
          // Mid-layout, or a tab that has just been hidden. The next
          // observation fits it.
        }
      });
      observer.observe(hostRef.current);

      // The app's theme can change under a canvas that cannot inherit
      // anything, so the colours are handed over again when it does.
      themeWatch = new MutationObserver(() => {
        if (hostRef.current) term.options.theme = xtermTheme(hostRef.current);
      });
      themeWatch.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["class", "data-theme", "style"],
      });
    })();

    return () => {
      alive = false;
      offEvent?.();
      observer?.disconnect();
      themeWatch?.disconnect();
      // The emulator goes; the shell stays. Closing the TAB closes the shell,
      // and that happens in `dropTab`, not here.
      try {
        term?.dispose();
      } catch {
        /* already gone */
      }
      fitRef.current = null;
    };
  }, [tab.id, tab.serial, cwd, job, onRename]);

  // Coming forward, the emulator has to re-measure: it was sized while hidden,
  // where every box is zero.
  React.useEffect(() => {
    if (hidden) return;
    const id = requestAnimationFrame(() => {
      try {
        fitRef.current?.fit();
      } catch {
        /* not laid out yet */
      }
    });
    return () => cancelAnimationFrame(id);
  }, [hidden]);

  return (
    <div hidden={hidden} className={cn("min-h-0 flex-1 flex-col", !hidden && "flex")}>
      <Header
        here={here}
        // Shown for a mirrored process only. A real terminal has Ctrl+C, which
        // is better in every way: it reaches the foreground program rather
        // than the shell. A background process has nothing of the sort, and
        // being told to ask the agent to stop your own dev server is absurd.
        busy={job && running}
        onStop={() => terminal.interrupt(tab.id).catch(() => {})}
        onClose={onClose}
      />
      {/* The padding is on the wrapper rather than on the canvas: xterm
          measures its host to decide how many columns fit, and padding inside
          it would be counted as room for characters that then get clipped. */}
      <div className="min-h-0 flex-1 overflow-hidden px-2 py-1">
        <div ref={hostRef} className="size-full" />
      </div>
    </div>
  );
}

/* ── the fallback: a log and a command box ─────────────────────────────────── */

/**
 * What a piped shell honestly is.
 *
 * Deliberately NOT dressed up as a terminal. There is no cursor to place and
 * no grid to repaint, so a field pinned under the output is the truthful
 * shape: history and editing come free from the browser, the text can be
 * selected, and nothing lies about where the cursor is.
 */
function PipeTab({ tab, cwd, hidden, onRename, onClose }) {
  const [text, setText] = React.useState("");
  const [line, setLine] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [here, setHere] = React.useState("");
  const [history, setHistory] = React.useState([]);
  const [recall, setRecall] = React.useState(-1);

  const logRef = React.useRef(null);
  const stuck = React.useRef(true);

  const atBottom = () => {
    const node = logRef.current;
    if (!node) return true;
    return node.scrollHeight - node.scrollTop - node.clientHeight < 40;
  };

  React.useLayoutEffect(() => {
    const node = logRef.current;
    // Only when the reader was already at the bottom. Yanking someone back
    // down while they are reading an error further up is the single most
    // irritating thing a log pane can do.
    if (node && stuck.current) node.scrollTop = node.scrollHeight;
  }, [text]);

  React.useEffect(() => {
    let alive = true;

    terminal
      .open(tab.id, cwd ? { cwd } : undefined)
      .then((state) => {
        if (!alive || !state) return;
        setText(state.scrollback ?? "");
        setHere(state.cwd ?? "");
        setBusy(Boolean(state.busy));
        onRename(tab.id, labelFor({ kind: "sh", serial: tab.serial, cwd: state.cwd }));
      })
      .catch((error) => {
        if (alive) setText(`The shell could not start.\n${error?.message ?? error}\n`);
      });

    const off = terminal.onEvent((event) => {
      if (event?.id !== tab.id) return;
      if (event.type === "data") {
        stuck.current = atBottom();
        setText((prev) => prev + event.text);
      } else if (event.type === "cwd") {
        // Where the shell ended up, which is the tab's label. This used to
        // read `done`, a type nothing has ever emitted, so a piped tab kept
        // the folder it was opened in no matter what it was told to cd into.
        setHere(event.cwd ?? "");
        onRename(tab.id, labelFor({ kind: "sh", serial: tab.serial, cwd: event.cwd }));
      } else if (event.type === "exit") {
        setBusy(false);
      }
    });

    return () => {
      alive = false;
      off?.();
    };
  }, [tab.id, tab.serial, cwd, onRename]);

  const send = () => {
    const command = line.trim();
    if (!command) return;
    setLine("");
    setRecall(-1);
    setHistory((prev) => [command, ...prev.filter((one) => one !== command)].slice(0, HISTORY));
    stuck.current = true;
    setBusy(true);
    terminal.run(tab.id, command).catch((error) => {
      setBusy(false);
      setText((prev) => `${prev}\n[${error?.message ?? error}]\n`);
    });
  };

  const onKeyDown = (event) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      send();
      return;
    }
    // Ctrl+C stops what is running rather than copying - which is what it does
    // in a terminal - with the selection-aware exception: if text is selected,
    // the person meant copy.
    if (event.key === "c" && (event.ctrlKey || event.metaKey)) {
      if (window.getSelection()?.toString()) return;
      event.preventDefault();
      terminal.interrupt(tab.id).catch(() => {});
      setBusy(false);
      return;
    }
    if (event.key === "ArrowUp" && history.length) {
      event.preventDefault();
      const next = Math.min(recall + 1, history.length - 1);
      setRecall(next);
      setLine(history[next]);
      return;
    }
    if (event.key === "ArrowDown" && recall >= 0) {
      event.preventDefault();
      const next = recall - 1;
      setRecall(next);
      setLine(next < 0 ? "" : history[next]);
    }
  };

  return (
    <div hidden={hidden} className={cn("min-h-0 flex-1 flex-col", !hidden && "flex")}>
      <Header
        here={here}
        busy={busy}
        onStop={() => {
          terminal.interrupt(tab.id).catch(() => {});
          setBusy(false);
        }}
        onClose={onClose}
      />

      <pre
        ref={logRef}
        onScroll={() => {
          stuck.current = atBottom();
        }}
        className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap wrap-break-word px-3 py-2 font-mono text-[12px] leading-relaxed text-foreground selection:bg-primary/30"
      >
        {text}
      </pre>

      <div className="flex shrink-0 items-center gap-2 border-t border-border-subtle px-3 py-2">
        <span aria-hidden="true" className="shrink-0 font-mono text-[12px] text-muted-foreground">
          &gt;
        </span>
        <input
          value={line}
          spellCheck={false}
          autoComplete="off"
          aria-label="Command"
          placeholder={busy ? "running…" : "npm run build"}
          className="min-w-0 flex-1 bg-transparent font-mono text-[12px] text-foreground outline-none placeholder:text-muted-foreground"
          onChange={(event) => setLine(event.target.value)}
          onKeyDown={onKeyDown}
        />
      </div>
    </div>
  );
}

// `px-2`, matching the browser pane's toolbar and the tab strip above both of
// them, so every control on this panel's right edge sits on one vertical line.
function Header({ here, busy = false, onStop, onClose }) {
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-border-subtle px-2 py-1.5">
      <p
        className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground"
        title={here}
      >
        {here || "starting…"}
      </p>
      {busy ? (
        <button
          type="button"
          className="shrink-0 rounded-sm fill-control px-2 py-0.5 text-[11px] text-muted-foreground"
          onClick={onStop}
        >
          Stop
        </button>
      ) : null}
      {onClose ? (
        <IconButton size="sm" aria-label="Close this terminal" onClick={onClose}>
          <X className="size-3.5" />
        </IconButton>
      ) : null}
    </div>
  );
}
