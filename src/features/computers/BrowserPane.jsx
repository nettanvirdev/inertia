import * as React from "react";
import { ArrowLeft, ArrowRight, Globe, RotateCw, X } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { IconButton } from "@/components/ui/icon-button";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Spinner } from "@/components/ui/spinner";
import { useToast } from "@/components/ui/toast";
import { computers as machines } from "@/lib/computers";

/**
 * A browser, running inside the machine.
 *
 * The tab strip here used to be live and the pages were not: eight fabricated
 * tabs with fabricated titles, and a viewport that explained it could not
 * render them. This one starts a real Chromium in the sandbox, on the sandbox's
 * own display, and shows frames of it.
 *
 * Why not an in-app webview? Because it would be *this* machine's browser, on
 * this machine's network, with this machine's cookies - which is the opposite
 * of what a sandboxed computer is for. The whole value of an agent's browser is
 * that it is somewhere else. So the page loads over there, and what crosses
 * back is a picture.
 *
 * Navigation goes through the keyboard, via xdotool, for the same reason: there
 * is no automation protocol attached to this Chromium and adding one would mean
 * a debugging port, a client and a dependency. Alt+Left is what a person would
 * press, and it is what gets sent.
 */

/** Frames while a page is being watched. Slower than the desktop pane's
 *  default, because a page that has loaded does not change. */
const FRAME_MS = 3000;

const START_PAGE = "https://duckduckgo.com";

export function BrowserPane({ computer }) {
  const { toast } = useToast();

  const [address, setAddress] = React.useState(START_PAGE);
  const [current, setCurrent] = React.useState(null);
  const [frame, setFrame] = React.useState(null);
  const [busy, setBusy] = React.useState(false);
  const [starting, setStarting] = React.useState(false);
  const [unavailable, setUnavailable] = React.useState(null);

  const running = computer.status === "running";

  React.useEffect(() => {
    setCurrent(null);
    setFrame(null);
    setUnavailable(null);
  }, [computer.id]);

  const capture = React.useCallback(async () => {
    if (!running || !current) return;
    setBusy(true);
    try {
      const shot = await machines.screenshot(computer.id);
      if (shot) setFrame(shot);
    } catch {
      // A dropped frame is not worth a toast; the next one is three seconds
      // away and the last one is still on screen.
    } finally {
      setBusy(false);
    }
  }, [computer.id, current, running]);

  React.useEffect(() => {
    if (!current) return undefined;
    capture();
    const timer = window.setInterval(capture, FRAME_MS);
    return () => window.clearInterval(timer);
  }, [current, capture]);

  if (!running) {
    return (
      <EmptyState
        className="min-h-0 flex-1"
        icon={Globe}
        title={`${computer.name} is ${computer.status}`}
        description="Start the machine to open a page on it."
      />
    );
  }

  /**
   * Everything this pane does to the machine goes through `computers.drive`.
   *
   * It used to build its own shell commands here, and the agent's computer tool
   * built its own separately - two launchers, two profile directories, two pid
   * files. They drifted, as duplicated commands do: the tool learned that
   * Chromium cannot nest its own sandbox inside Docker Desktop and this pane
   * did not, so the pane went on failing with a message about installing a
   * package that does not fix it, long after the same problem was solved a file
   * away. One place now, and both callers get every fix.
   */
  async function drive(action, args) {
    return machines.drive(computer.id, action, args);
  }

  async function open(url) {
    const target = /^[a-z]+:\/\//i.test(url) ? url : `https://${url}`;
    setStarting(true);
    setUnavailable(null);
    try {
      const result = await drive("open", { url: target });

      if (result.unsupported) {
        setUnavailable(
          `${result.text} The image in sandbox/ carries a browser and a display; ` +
            "a machine built from a plain base image does not."
        );
        return;
      }
      if (!result.ok) {
        setUnavailable(`The browser did not start on ${computer.name}. ${result.text.slice(0, 400)}`);
        return;
      }

      setCurrent(target);
      setAddress(target);
    } catch (error) {
      toast({ variant: "error", title: "Could not open the page", description: error?.message });
    } finally {
      setStarting(false);
    }
  }

  /** A keystroke into whatever window has focus on the machine's display. */
  async function key(combo) {
    try {
      await drive("key", { key: combo });
      window.setTimeout(capture, 800);
    } catch (error) {
      toast({ variant: "error", title: "That key did not reach the machine" });
    }
  }

  async function close() {
    await drive("close", {});
    setCurrent(null);
    setFrame(null);
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex shrink-0 items-center gap-1.5">
        <IconButton size="lg" label="Back" disabled={!current} onClick={() => key("alt+Left")}>
          <ArrowLeft />
        </IconButton>
        <IconButton
          size="lg"
          label="Forward"
          disabled={!current}
          onClick={() => key("alt+Right")}
        >
          <ArrowRight />
        </IconButton>
        <IconButton size="lg" label="Reload" disabled={!current} onClick={() => key("F5")}>
          <RotateCw />
        </IconButton>

        <form
          className="min-w-0 flex-1"
          onSubmit={(event) => {
            event.preventDefault();
            open(address.trim());
          }}
        >
          <Input
            size="sm"
            value={address}
            onChange={(event) => setAddress(event.target.value)}
            aria-label={`Address to open on ${computer.name}`}
            placeholder="Type a URL and press Enter"
            spellCheck={false}
            autoComplete="off"
          />
        </form>

        {current ? (
          <IconButton size="lg" label="Close the browser" onClick={close}>
            <X />
          </IconButton>
        ) : null}
      </div>

      <div className="relative min-h-52 flex-1 overflow-hidden rounded-2xl bg-card-darker">
        {unavailable ? (
          <div className="flex size-full flex-col items-center justify-center gap-2 px-8 text-center">
            <Globe className="size-7 text-muted-foreground/60" aria-hidden="true" />
            <p className="max-w-96 text-[12px] leading-relaxed text-muted-foreground">
              {unavailable}
            </p>
          </div>
        ) : starting ? (
          <div className="flex size-full flex-col items-center justify-center gap-3">
            <Spinner />
            <p className="text-[12px] text-muted-foreground">
              Starting a browser on {computer.name}…
            </p>
          </div>
        ) : !current ? (
          <div className="flex size-full flex-col items-center justify-center gap-3 px-8 text-center">
            <Globe className="size-7 text-muted-foreground/60" aria-hidden="true" />
            <p className="text-[13px] text-muted-foreground">
              Nothing is open on {computer.name}.
            </p>
            <Button size="sm" variant="subtle" onClick={() => open(address || START_PAGE)}>
              Open a page
            </Button>
          </div>
        ) : frame ? (
          <img
            src={frame}
            alt={`${current} on ${computer.name}`}
            className="size-full object-contain"
          />
        ) : (
          <div className="flex size-full items-center justify-center">
            <Spinner />
          </div>
        )}

        {current && frame ? (
          <span className="absolute top-3 left-3 inline-flex h-6 items-center gap-1.5 rounded-full fill-secondary px-2.5 text-[11px] text-muted-foreground">
            <span
              aria-hidden="true"
              className={cn(
                "size-1.5 rounded-full",
                busy ? "bg-muted-foreground/50" : "animate-soft-pulse bg-success"
              )}
            />
            Live from {computer.name}
          </span>
        ) : null}
      </div>

      <p className="shrink-0 text-[11px] text-muted-foreground">
        The page loads on the machine, not here - its network, its cookies. What you see is a
        frame of its screen every {FRAME_MS / 1000} seconds; the buttons above send keystrokes
        to it.
      </p>
    </div>
  );
}
