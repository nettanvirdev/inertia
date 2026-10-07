import * as React from "react";
import { Camera, Eye, Hand, Maximize2, Monitor, Play, Pause } from "@/components/icons";
import { cn } from "@/lib/utils";
import { COMPUTER_STATUS_META } from "@/data/computers";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Select } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { useToast } from "@/components/ui/toast";
import { computers as machines } from "@/lib/computers";

/**
 * The machine's screen, live where the machine can serve one.
 *
 * Two modes, and which one you get is decided by the machine rather than by a
 * preference. A container built from our image runs a VNC server on its display
 * and a noVNC client in front of it, so the pane is the real screen at real
 * speed and a person can take the mouse. Anything else - a local machine, a
 * Daytona sandbox, a container from before the image had a VNC server in it -
 * still has `import`, so the pane falls back to still frames.
 *
 * The fallback is not a lesser version of the same thing. Frames answer "what
 * is on screen" over any transport that can run a command, which is why they
 * stay: they are the reason this pane works at all on a provider we have never
 * met. The live view answers the other question, the one a person asks when the
 * agent is stuck, which is "let me do it myself".
 *
 * ## Watching and driving are different connections
 *
 * The image runs two VNC servers: 6080 is view-only, 6081 is not. Switching to
 * control changes which port the iframe is pointed at, so "you are only
 * watching" is a fact about the socket rather than about a URL parameter the
 * page could be talked out of. Both are published to loopback on the host, and
 * both ask for the machine's own password, which arrives in the URL's fragment
 * (`#password=`) so it is never sent to the server; `novncUrl` keeps it.
 */

const RATES = [
  { value: "0", label: "Paused" },
  { value: "2000", label: "Every 2s" },
  { value: "5000", label: "Every 5s" },
  { value: "15000", label: "Every 15s" },
];

/**
 * `resize=scale` fits the machine's 1600x900 into whatever the pane is, and
 * `reconnect` is what makes a restart look like a pause rather than a failure.
 * `view_only` is belt to the braces of connecting to the view-only port.
 */
function novncUrl(base, { controlling }) {
  const url = new URL(base);
  url.searchParams.set("autoconnect", "1");
  url.searchParams.set("resize", "scale");
  url.searchParams.set("reconnect", "1");
  url.searchParams.set("reconnect_delay", "1500");
  url.searchParams.set("show_dot", "1");
  if (!controlling) url.searchParams.set("view_only", "1");
  return url.toString();
}

/**
 * `initialControlling` is for the dialog beside the chat, which opens either
 * to watch or to drive; the pane still owns the toggle after that. It is
 * read once, on mount and on a change of machine, so a dialog that was
 * opened with "Take control" and then handed the mouse back stays that way.
 */
export function DesktopPane({ computer, onStart, initialControlling = false }) {
  const { toast } = useToast();
  const frameRef = React.useRef(null);

  const [live, setLive] = React.useState(undefined);
  const [controlling, setControlling] = React.useState(initialControlling);
  const [frame, setFrame] = React.useState(null);
  const [rate, setRate] = React.useState("5000");
  const [busy, setBusy] = React.useState(false);
  const [noDisplay, setNoDisplay] = React.useState(false);
  const [takenAt, setTakenAt] = React.useState(null);

  const running = computer.status === "running";
  const statusLabel =
    COMPUTER_STATUS_META[computer.status]?.label?.toLowerCase() ?? computer.status;

  const capture = React.useCallback(async () => {
    if (!running) return;
    setBusy(true);
    try {
      const shot = await machines.screenshot(computer.id);
      if (shot) {
        setFrame(shot);
        setNoDisplay(false);
        setTakenAt(new Date());
      } else {
        // Null is the honest answer for a machine with no X server, and it is
        // permanent for that machine - so polling stops rather than asking the
        // same question every five seconds forever.
        setNoDisplay(true);
        setRate("0");
      }
    } catch (error) {
      toast({ variant: "error", title: "Could not take a frame", description: error?.message });
      setRate("0");
    } finally {
      setBusy(false);
    }
  }, [computer.id, running, toast]);

  // A different machine is a different screen.
  React.useEffect(() => {
    setLive(undefined);
    setControlling(initialControlling);
    setFrame(null);
    setNoDisplay(false);
    setTakenAt(null);
    // Only the machine resets this; the opening mode is read at that moment.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [computer.id]);

  // Ask once per running machine whether there is a live screen to watch. A
  // machine that has none answers null, and that is the answer for as long as
  // it is up - the ports are published when it is made and do not appear later.
  React.useEffect(() => {
    if (!running) return undefined;
    let alive = true;
    machines
      .screen(computer.id)
      .then((found) => {
        if (alive) setLive(found ?? null);
      })
      .catch(() => {
        if (alive) setLive(null);
      });
    return () => {
      alive = false;
    };
  }, [computer.id, running]);

  // Frames are only polled when there is no live view. Doing both would run a
  // screenshot command every few seconds against a machine whose screen is
  // already on the glass, which is work nobody asked for.
  React.useEffect(() => {
    if (!running || live === undefined || live) return undefined;
    capture();
    const ms = Number(rate);
    if (!ms) return undefined;
    const timer = window.setInterval(capture, ms);
    return () => window.clearInterval(timer);
  }, [running, rate, capture, live]);

  function fullscreen() {
    const el = frameRef.current;
    if (!el?.requestFullscreen) {
      toast({ title: "Fullscreen unavailable", description: "This window does not allow it." });
      return;
    }
    if (document.fullscreenElement) document.exitFullscreen();
    else el.requestFullscreen().catch(() => toast({ title: "Fullscreen was blocked" }));
  }

  const screenUrl = live ? novncUrl(controlling ? live.control : live.view, { controlling }) : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <div
        ref={frameRef}
        className="relative min-h-52 w-full flex-1 overflow-hidden rounded-2xl bg-card-darker"
      >
        {!running ? (
          <div className="flex size-full flex-col items-center justify-center gap-3 px-8 text-center">
            <Monitor className="size-7 text-muted-foreground/60" aria-hidden="true" />
            <p className="text-[13px] text-muted-foreground">
              {computer.name} is {statusLabel}.
            </p>
            {onStart ? (
              <Button size="sm" variant="subtle" onClick={onStart}>
                <Play />
                Start it
              </Button>
            ) : null}
          </div>
        ) : screenUrl ? (
          // Keyed on the URL so switching between watching and controlling
          // opens a new connection to the other port rather than trying to
          // change the policy of one that is already open.
          <iframe
            key={screenUrl}
            src={screenUrl}
            title={`The screen of ${computer.name}`}
            className="size-full border-0 bg-black"
            allow="clipboard-read; clipboard-write"
          />
        ) : live === undefined ? (
          <div className="flex size-full items-center justify-center">
            <Spinner />
          </div>
        ) : noDisplay ? (
          <div className="flex size-full flex-col items-center justify-center gap-2 px-8 text-center">
            <Monitor className="size-7 text-muted-foreground/60" aria-hidden="true" />
            <p className="text-[13px] text-muted-foreground">This machine has no display.</p>
            <p className="max-w-96 text-[11px] leading-relaxed text-muted-foreground/70">
              Nothing is drawing a screen inside it. The image in{" "}
              <span className="font-mono">sandbox/</span> installs an X server and a window
              manager; a machine built from a plain base image will not have them.
            </p>
          </div>
        ) : frame ? (
          <img
            src={frame}
            alt={`The screen of ${computer.name}`}
            className="size-full object-contain"
          />
        ) : (
          <div className="flex size-full items-center justify-center">
            <Spinner />
          </div>
        )}

        {running && (screenUrl || frame) ? (
          <span className="absolute top-3 left-3 inline-flex h-6 items-center gap-1.5 rounded-full fill-secondary px-2.5 text-[11px] text-muted-foreground">
            <span
              aria-hidden="true"
              className={cn(
                "size-1.5 rounded-full",
                screenUrl && controlling
                  ? "bg-warning"
                  : screenUrl || Number(rate)
                    ? "animate-soft-pulse bg-success"
                    : "bg-muted-foreground/50"
              )}
            />
            {screenUrl
              ? controlling
                ? "You have control"
                : "Live"
              : takenAt
                ? takenAt.toLocaleTimeString()
                : "Watching"}
          </span>
        ) : null}
      </div>

      <div className="flex shrink-0 flex-wrap items-center gap-2">
        {screenUrl ? (
          <Button
            size="sm"
            variant={controlling ? "default" : "subtle"}
            onClick={() => setControlling((was) => !was)}
          >
            {controlling ? <Eye /> : <Hand />}
            {controlling ? "Stop controlling" : "Take control"}
          </Button>
        ) : (
          <>
            <Select
              size="sm"
              ariaLabel="How often to take a frame"
              value={rate}
              onChange={setRate}
              disabled={!running || noDisplay}
              // `options`, not `items`. Select ignores an unknown prop, so this
              // rendered as a control with nothing in it and no error anywhere -
              // the frame rate simply could not be changed.
              options={RATES}
              className="w-36"
            />
            <Button size="sm" variant="subtle" onClick={capture} disabled={!running || busy}>
              {Number(rate) ? <Pause /> : <Camera />}
              {busy ? "Taking…" : "Take a frame"}
            </Button>
          </>
        )}
        <IconButton
          size="lg"
          label="Fullscreen"
          onClick={fullscreen}
          disabled={!screenUrl && !frame}
        >
          <Maximize2 />
        </IconButton>
        <p className="min-w-0 flex-1 text-right text-[11px] text-muted-foreground">
          {screenUrl
            ? controlling
              ? "Your mouse and keyboard are on the machine. The agent is still working - take control mid-task and you will fight it for the pointer."
              : "Live, and watching only. Take control to use the mouse and keyboard yourself."
            : "Still frames, not a stream. Nothing here takes control of the machine - use the terminal for that."}
        </p>
      </div>
    </div>
  );
}
