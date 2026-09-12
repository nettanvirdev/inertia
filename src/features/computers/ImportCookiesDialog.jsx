import * as React from "react";
import { Cloud, Download, Globe, ShieldCheck } from "@/components/icons";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RadioGroup, RadioItem } from "@/components/ui/radio";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { useToast } from "@/components/ui/toast";
import { relativeTime } from "@/data";
import { computers as machines } from "@/lib/computers";
import { preview } from "@/lib/preview";

/**
 * Bring a signed-in session from this desktop into a browser the agent drives.
 *
 * The agent's browser starts signed in to nothing, so every task that touches
 * an account begins with a login the agent cannot safely do. This is the way
 * out that the web already has: copy the cookies the user is already carrying,
 * once, because they asked. No password is typed and no second factor is
 * needed - the browser simply is the user, on the sites they pick.
 *
 * There is no agent tool behind this. Moving the user's live sessions is
 * something the user does from here, deliberately, and nothing an agent can ask
 * for on its own.
 *
 * ── Two browsers, one dialog ───────────────────────────────────────────────
 * The machine's browser in a container and the pane beside the chat are two
 * places the same sessions can go, and the questions - which profile, which
 * sites, what happened - are the same for both. So the dialog takes a
 * `target`: a name to say, a way to list the profiles, and a way to run the
 * import. `forComputer` and `forPreview` below build the two that exist.
 */
export function ImportCookiesDialog({ open, onOpenChange, target }) {
  const { toast } = useToast();

  const [sources, setSources] = React.useState(null);
  const [error, setError] = React.useState(null);
  const [picked, setPicked] = React.useState("");
  const [domains, setDomains] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [result, setResult] = React.useState(null);

  // Load the profiles each time the dialog opens - a browser signed into a new
  // account since last time should be here without a restart.
  React.useEffect(() => {
    if (!open) return undefined;
    let live = true;
    setSources(null);
    setError(null);
    setResult(null);
    setDomains("");
    target
      .sources()
      .then((found) => {
        if (!live) return;
        const usable = found.filter((entry) => entry.cookies !== 0);
        setSources(usable);
        setPicked(usable[0]?.id ?? "");
      })
      .catch((problem) => live && setError(problem?.message ?? String(problem)));
    return () => {
      live = false;
    };
    // The target is rebuilt by its owner on every render; what matters is
    // which one it is, and that is `open` flipping.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const chosen = sources?.find((entry) => entry.id === picked);

  async function run() {
    if (!chosen) return;
    setBusy(true);
    setResult(null);
    try {
      const outcome = await target.run({
        sourceId: chosen.id,
        domains: domains.trim() || undefined,
      });
      setResult(outcome);
      if (outcome.ok) {
        toast({
          variant: "success",
          title: `${outcome.imported} cookie${outcome.imported === 1 ? "" : "s"} on ${target.name}`,
          description: `From ${outcome.from.browser} · ${outcome.from.profile}.`,
        });
        target.after?.(outcome);
      }
    } catch (problem) {
      toast({
        variant: "error",
        title: "The import did not finish",
        description: problem?.message ?? String(problem),
      });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange} size="md">
      <DialogTitle>Bring your sessions to {target.name}</DialogTitle>
      <DialogDescription>
        Copy the sites you are signed in to from a browser on this computer, so {target.name}{" "}
        opens them already logged in. Nothing is typed and no password leaves this computer -
        only the cookies you already have.
      </DialogDescription>

      <DialogBody className="flex flex-col gap-4">
        {error ? (
          <p className="animate-fade-in rounded-xl bg-destructive-wash px-3 py-2.5 text-[12px] text-destructive-ink">
            {error}
          </p>
        ) : sources === null ? (
          <div className="flex items-center justify-center gap-2 py-8 text-[13px] text-muted-foreground">
            <Spinner />
            Looking for browsers on this computer…
          </div>
        ) : sources.length === 0 ? (
          <div className="flex flex-col items-center gap-2 py-8 text-center">
            <Globe className="size-7 text-muted-foreground/60" aria-hidden="true" />
            <p className="max-w-80 text-[13px] text-muted-foreground">
              No browser with readable cookies was found on this computer. Sign in to a site in
              Chrome, Edge, Firefox, Brave or Safari, then try again.
            </p>
          </div>
        ) : (
          <>
            <div>
              <p className="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
                From
              </p>
              <ScrollArea className="max-h-[min(20rem,calc(100dvh-26rem))] -mx-1 px-1">
                <RadioGroup value={picked} onChange={setPicked} label="Browser profile to copy from">
                  {sources.map((entry) => (
                    <RadioItem
                      key={entry.id}
                      value={entry.id}
                      label={
                        <span className="flex items-baseline gap-2">
                          <span className="text-[13px] text-foreground">{entry.browser}</span>
                          <span className="text-[12px] text-muted-foreground">{entry.profile}</span>
                        </span>
                      }
                      description={sourceLine(entry)}
                    />
                  ))}
                </RadioGroup>
              </ScrollArea>
            </div>

            <div>
              <label
                htmlFor="cookie-domains"
                className="mb-1.5 block text-[11px] font-medium tracking-wide text-muted-foreground uppercase"
              >
                Only these sites (optional)
              </label>
              <Input
                id="cookie-domains"
                size="sm"
                value={domains}
                onChange={(event) => setDomains(event.target.value)}
                placeholder="google.com, github.com — leave empty for all"
                spellCheck={false}
                autoComplete="off"
              />
              <p className="mt-1.5 text-[11px] text-muted-foreground">
                Narrowing to the sites the agent actually needs is the safer choice: everything
                else stays on this computer.
              </p>
            </div>

            {result ? <Outcome result={result} machineName={target.name} /> : null}
          </>
        )}
      </DialogBody>

      <DialogFooter>
        <div className="mr-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <ShieldCheck className="size-3.5" aria-hidden="true" />
          Read here, written only to {target.name}.
        </div>
        <Button size="pill" variant="secondary" onClick={() => onOpenChange(false)}>
          {result?.ok ? "Done" : "Cancel"}
        </Button>
        <Button size="pill" onClick={run} disabled={busy || !chosen}>
          {busy ? (
            <>
              <Spinner className="size-4" /> Copying…
            </>
          ) : (
            <>
              <Download className="size-4" aria-hidden="true" /> Copy sessions
            </>
          )}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

/**
 * The machine's browser, in its container.
 *
 * Built fresh per render and cheap to: three closures over a computer record.
 */
export function forComputer(computer) {
  return {
    name: computer.name,
    sources: () => machines.cookieSources(),
    run: (options) => machines.importCookies(computer.id, options),
  };
}

/**
 * The browser pane beside the chat.
 *
 * Every tab shares one session, so the import is for the pane rather than
 * for a tab - but the tab that asked is the one whose page is now stale, and
 * `after` reloads it so the person sees themselves signed in without having
 * to know that they should press reload.
 */
export function forPreview(tabId) {
  return {
    name: "the browser beside this chat",
    sources: () => preview.cookieSources(),
    run: (options) => preview.importCookies(options),
    after: () => {
      if (tabId) preview.reload(tabId).catch(() => {});
    },
  };
}

/** The line under a profile: how many cookies, and when it last changed. */
function sourceLine(entry) {
  const count =
    entry.cookies == null
      ? "cookies could not be counted"
      : `${entry.cookies} cookie${entry.cookies === 1 ? "" : "s"}`;
  return entry.updatedAt ? `${count} · updated ${relativeTime(entry.updatedAt)}` : count;
}

/**
 * What came back, said plainly.
 *
 * The two numbers that matter are the ones that went and the ones that could
 * not, and the second needs its reason or it reads as a silent failure. A
 * sealed store is the common one - Chrome and Edge 127 and later - and it is
 * not a bug to be fixed here but a fact to be stated, with Firefox or Brave as
 * the way around it.
 */
function Outcome({ result, machineName }) {
  if (!result.ok) {
    return (
      <div className="animate-slide-up rounded-xl fill-whisper px-3 py-2.5 text-[12px] text-muted-foreground">
        {result.reason === "nothing-readable" ? (
          <>
            None of this profile's cookies could be read on this computer.{" "}
            {topReason(result.reasons)}
          </>
        ) : result.reason === "no-profile" ? (
          <>The machine's browser has not started yet, so there was nowhere to put them.</>
        ) : result.reason === "nothing-accepted" ? (
          <>The browser refused every one of them. {topReason(result.reasons)}</>
        ) : (
          <>The cookies could not be written to {machineName}.</>
        )}
      </div>
    );
  }

  const left = unreadable(result.skipped);
  return (
    <div className="flex animate-slide-up flex-col gap-1.5 rounded-xl bg-success-wash px-3 py-2.5">
      <p className="flex items-center gap-2 text-[13px] text-success-ink">
        <Cloud className="size-4" aria-hidden="true" />
        {result.imported} cookie{result.imported === 1 ? "" : "s"} now on {machineName}.
      </p>
      {result.browserClosed ? (
        <p className="text-[11px] text-muted-foreground">
          The machine's browser was restarted so it would pick them up.
        </p>
      ) : null}
      {left ? (
        <p className="text-[11px] text-muted-foreground">
          {left} were left behind. {topReason(result.reasons)}
        </p>
      ) : null}
    </div>
  );
}

function unreadable(skipped) {
  return skipped?.unreadable ?? 0;
}

/** The most common reason a value could not be read, as a sentence. */
function topReason(reasons) {
  const top = reasons?.[0]?.reason;
  if (!top) return "";
  return `Most were ${top}. A Firefox or Brave profile is usually readable when Chrome is not.`;
}
