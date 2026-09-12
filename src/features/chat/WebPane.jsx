import * as React from "react";
import { ArrowLeft, ArrowRight, Globe, Icon, KeyRound, RotateCw, X } from "@/components/icons";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";
import { isPreviewAvailable, preview } from "@/lib/preview";
import { isShowable, rectOf } from "@/features/chat/pane-rect.js";
import { activeOf, addTab, adoptTab, closeTab, labelFor } from "@/features/chat/pane-tabs.js";
import { clearReveal, onRevealPane } from "@/features/chat/pane-reveal";
import { keepPane, retirePane } from "@/features/chat/pane-retire";
import { isPaneDragging, onPaneDrag } from "@/features/chat/pane-drag";
import { PaneTabStrip } from "@/features/chat/PaneTabStrip";
import { ImportCookiesDialog, forPreview } from "@/features/computers/ImportCookiesDialog";

/**
 * A browser, beside the conversation, with tabs.
 *
 * The unusual thing about this component is that it does not render the page.
 * It cannot: each page is a Chromium view the main process parks over this
 * window, native and outside the DOM. So what this renders is the CHROME - the
 * tab strip, an address bar, back and forward - and, below it, a hole: an
 * empty box whose only job is to be measured and reported, so the view can be
 * put exactly there.
 *
 * ── The bug that cost the whole pane, and is now impossible ────────────────
 * The measurement is `getBoundingClientRect()`, which returns a `DOMRect` - an
 * object whose numbers live on its prototype rather than on itself. The
 * context bridge does not clone host objects, so the main process received
 * `{}`, the bounds clamped to zero, and every browser was created, navigated,
 * and never once made visible. The page loaded perfectly and was drawn
 * nowhere, which reads exactly like a browser that cannot load anything. See
 * `pane-rect.js`; the conversion is a named, tested function now.
 *
 * ── What is mounted and what is visible ───────────────────────────────────
 * Every tab stays mounted. A background tab's hole measures to nothing, so its
 * view is hidden while its page keeps running - which is what a tab is.
 * Unmounting would instead leave a live view floating over the transcript with
 * nothing left to tell it where to be.
 *
 * Nothing in the app can be drawn ON TOP of a hole: a native view is above
 * every pixel of the window. That is why the chrome sits above the hole rather
 * than floating on it.
 */
export function WebPane({ paneId, onClose, className }) {
  const available = isPreviewAvailable();

  const [tabs, setTabs] = React.useState(() => addTab([], { threadId: paneId, kind: "web" }).tabs);
  const [active, setActive] = React.useState(() => `chat:${paneId}:web:1`);
  const current = activeOf(tabs, active);

  const openTab = () => {
    const next = addTab(tabs, { threadId: paneId, kind: "web" });
    setTabs(next.tabs);
    setActive(next.active);
  };

  const dropTab = (id) => {
    const next = closeTab(tabs, current, id);
    // The view belongs to the main process and outlives this component, so a
    // closed tab has to be closed there too, or it is a page nobody can see
    // and nobody can stop.
    preview.close(id).catch(() => {});
    if (!next.tabs.length) {
      onClose?.();
      return;
    }
    setTabs(next.tabs);
    setActive(next.active);
  };

  /** A tab renames itself once its page says what it is. */
  const rename = React.useCallback((id, label) => {
    setTabs((prev) =>
      prev.map((tab) => (tab.id === id && tab.label !== label ? { ...tab, label } : tab))
    );
  }, []);

  // A tool drove a tab. Adopt it if the agent reached a tab id this pane does
  // not have - which is what happens when it navigates before the pane has
  // ever been opened - and bring it to the front either way.
  React.useEffect(
    () =>
      onRevealPane(({ dock, tabId, threadId }) => {
        if (dock !== "web" || threadId !== paneId) return;
        setTabs((prev) => adoptTab(prev, tabId, "web"));
        setActive(tabId);
        // Handled. It waits until a pane says so, because until one is mounted
        // there is nobody to honour it - but a request kept for ever would
        // pull this panel back to it every time it is reopened.
        clearReveal("web", paneId, tabId);
      }),
    [paneId]
  );

  // Every view this pane opened goes when the pane does, or a conversation
  // switched away from leaves its browsers running with nothing to close them.
  // Scheduled rather than done, because an unmount is not proof of a goodbye -
  // see `pane-retire.js`, and the page this used to destroy mid-navigation.
  const live = React.useRef([]);
  live.current = tabs.map((tab) => tab.id);
  React.useEffect(
    () => () => {
      for (const id of live.current) retirePane(id, () => preview.close(id).catch(() => {}));
    },
    []
  );

  if (!available) {
    return (
      <div className={cn("flex min-h-0 flex-1 items-center justify-center p-6", className)}>
        <p className="text-center text-[12px] leading-relaxed text-muted-foreground">
          The browser pane needs the desktop app. This is the web preview, which has no browser of
          its own to lend.
        </p>
      </div>
    );
  }

  return (
    <div className={cn("flex min-h-0 flex-1 flex-col", className)}>
      <PaneTabStrip
        tabs={tabs}
        active={current}
        onSelect={setActive}
        onClose={dropTab}
        onAdd={openTab}
        addLabel="New browser tab"
      />
      {tabs.map((tab) => (
        <WebTab
          key={tab.id}
          tab={tab}
          hidden={tab.id !== current}
          onRename={rename}
          onClose={() => dropTab(tab.id)}
        />
      ))}
    </div>
  );
}

/* ── one tab ───────────────────────────────────────────────────────────────── */

function WebTab({ tab, hidden, onRename, onClose }) {
  const holeRef = React.useRef(null);
  // Hidden for the length of a dock drag. A native view is above the document,
  // so while one is on screen the handle under it cannot be dragged at all.
  const [dragging, setDragging] = React.useState(isPaneDragging);
  React.useEffect(() => onPaneDrag(setDragging), []);
  const [address, setAddress] = React.useState("");
  const [state, setState] = React.useState({ url: "", title: "", loading: false, error: null });
  const typing = React.useRef(false);
  // The person's own sessions, brought into the pane. A dialog, not a tool:
  // see ImportCookiesDialog for why the agent cannot ask for this.
  const [cookiesOpen, setCookiesOpen] = React.useState(false);

  /* -- where the view goes ------------------------------------------------ */

  // Read by the interval and the observer below, whose closure is made once
  // and must not be looking at the first render's values for ever.
  const visible = React.useRef(!hidden);

  React.useEffect(() => {
    const node = holeRef.current;
    if (!node) return undefined;

    let alive = true;
    const report = () => {
      if (!alive) return;
      // Converted, not passed. A DOMRect does not survive the bridge - see the
      // note at the top of this file.
      const rect = node.getBoundingClientRect();
      preview.place(tab.id, rectOf(rect), visible.current && isShowable(rect));
    };

    // Every source of movement, and there are more than there look: the dock's
    // own drag, the window resizing, the rail expanding, a tab coming forward.
    const observer = new ResizeObserver(report);
    observer.observe(node);
    window.addEventListener("resize", report);
    window.addEventListener("scroll", report, true);
    // The backstop. A ResizeObserver does not fire for a box that moved
    // without changing size, which is what happens when a sibling pane opens
    // above this one.
    const timer = setInterval(report, 400);
    report();

    return () => {
      alive = false;
      observer.disconnect();
      window.removeEventListener("resize", report);
      window.removeEventListener("scroll", report, true);
      clearInterval(timer);
      // The view outlives this component, so it has to be told to stop drawing
      // itself over whatever replaces us.
      preview.place(tab.id, { x: 0, y: 0, width: 0, height: 0 }, false);
    };
  }, [tab.id]);

  // A tab that has just come forward is placed now rather than at the next
  // tick of the timer above, or switching tabs has a visible lag.
  React.useLayoutEffect(() => {
    visible.current = !hidden && !dragging;
    const node = holeRef.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    preview.place(tab.id, rectOf(rect), visible.current && isShowable(rect));
  }, [tab.id, hidden, dragging]);

  /* -- what the page is doing --------------------------------------------- */

  React.useEffect(() => {
    // We are here, so this view is not orphaned - whatever the last unmount
    // thought.
    keepPane(tab.id);
    preview
      .open(tab.id)
      .then((next) => {
        if (!next) return;
        setState(next);
        setAddress(next.url ?? "");
      })
      .catch((error) => setState((prev) => ({ ...prev, error: error?.message ?? String(error) })));

    return preview.onEvent((payload) => {
      if (payload?.id !== tab.id) return;
      setState(payload);
      // The address bar follows the page, unless the person is typing in it -
      // a bar that rewrites itself under the cursor is unusable.
      if (!typing.current) setAddress(payload.url ?? "");
      onRename(
        tab.id,
        labelFor({ kind: "web", serial: tab.serial, title: payload.title, url: payload.url })
      );
    });
  }, [tab.id, tab.serial, onRename]);

  const go = (value) => {
    const target = String(value ?? address).trim();
    if (!target) return;
    typing.current = false;
    preview.navigate(tab.id, target).catch((error) => {
      setState((prev) => ({ ...prev, error: error?.message ?? String(error) }));
    });
  };

  return (
    <div hidden={hidden} className={cn("min-h-0 flex-1 flex-col", !hidden && "flex")}>
      <div className="flex shrink-0 items-center gap-1 border-b border-border-subtle px-2 py-1.5">
        <IconButton
          size="sm"
          aria-label="Back"
          disabled={!state.canGoBack}
          onClick={() => preview.back(tab.id).catch(() => {})}
        >
          <ArrowLeft className="size-3.5" />
        </IconButton>
        <IconButton
          size="sm"
          aria-label="Forward"
          disabled={!state.canGoForward}
          onClick={() => preview.forward(tab.id).catch(() => {})}
        >
          <ArrowRight className="size-3.5" />
        </IconButton>
        <IconButton
          size="sm"
          aria-label={state.loading ? "Stop" : "Reload"}
          onClick={() =>
            (state.loading ? preview.stop(tab.id) : preview.reload(tab.id)).catch(() => {})
          }
        >
          {state.loading ? <X className="size-3.5" /> : <RotateCw className="size-3.5" />}
        </IconButton>

        <Input
          value={address}
          spellCheck={false}
          placeholder="localhost:5173"
          aria-label="Address"
          className="h-7 min-w-0 flex-1 text-[12px]"
          onChange={(event) => {
            typing.current = true;
            setAddress(event.target.value);
          }}
          onBlur={() => {
            typing.current = false;
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") go(event.currentTarget.value);
            if (event.key === "Escape") {
              typing.current = false;
              setAddress(state.url ?? "");
              event.currentTarget.blur();
            }
          }}
        />

        <IconButton
          size="sm"
          aria-label="Sign in with your browser's sessions"
          onClick={() => setCookiesOpen(true)}
        >
          <KeyRound className="size-3.5" />
        </IconButton>
        <IconButton
          size="sm"
          aria-label="Open developer tools"
          onClick={() => preview.devTools(tab.id, true).catch(() => {})}
        >
          <Icon name="Code" className="size-3.5" />
        </IconButton>
        <IconButton size="sm" aria-label="Close this tab" onClick={onClose}>
          <X className="size-3.5" />
        </IconButton>
      </div>

      <ImportCookiesDialog
        open={cookiesOpen}
        onOpenChange={setCookiesOpen}
        target={forPreview(tab.id)}
      />

      {state.error ? (
        <p className="shrink-0 border-b border-border-subtle bg-destructive-surface px-3 py-1.5 text-[11px] text-destructive-ink">
          {state.error}
        </p>
      ) : null}

      {/* The hole. Deliberately empty: anything inside it is painted under a
          native view and never seen. The placeholder shows only before a page
          has ever loaded, which is the one moment the view is not covering
          this. */}
      {/* `ml-1` is the drag handle's half. The handle straddles the dock's
          edge, and a native view drawn flush to that edge covers the half of
          it that is inside the dock - which is most of what there is to aim
          at. Four pixels of the dock's own colour is the difference between a
          resizable panel and one that only resizes level with its toolbar. */}
      <div ref={holeRef} className="relative ml-1 min-h-0 flex-1 bg-white">
        {!state.url ? (
          <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-2 bg-background">
            <Globe className="size-5 text-muted-foreground" />
            <p className="text-[12px] text-muted-foreground">Type an address to open a page.</p>
          </div>
        ) : null}
        {state.loading ? (
          <div className="pointer-events-none absolute right-2 top-2">
            <Spinner className="size-3.5" />
          </div>
        ) : null}
      </div>
    </div>
  );
}
