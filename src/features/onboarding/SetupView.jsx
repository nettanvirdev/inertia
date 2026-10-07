import * as React from "react";
import {
  ChevronRight,
  CircleAlert,
  FolderCheck,
  FolderOpen,
  FolderPlus,
  FolderTree,
  TriangleAlert,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { useWorkspace } from "@/lib/workspace";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { ScrollArea } from "@/components/ui/scroll-area";

/**
 * First run, and the only screen that exists before there is a workspace.
 *
 * The whole app is one folder, so this question is asked once and never again -
 * which is exactly why it is a page and not a modal. The user is choosing where
 * their conversations, agents and keys will live for as long as they use the
 * app, and the second half of the answer is that pointing a fresh install at an
 * existing folder is the restore path. Both of those need room to be said in
 * plain words, so the copy leads and the field follows.
 *
 * The verdict under the field is the load-bearing part. Creating a folder,
 * adding to a folder someone already uses, and adopting a workspace that is
 * already there are three genuinely different outcomes, and the user has to be
 * able to tell which one the button is about to do before they press it.
 */

const DEBOUNCE_MS = 300;

// The glyph carries the verdict as much as the sentence does, so each one is
// the thing that is about to happen to a folder rather than a decoration: a
// folder gaining something, a folder already good, a folder to be careful with.
const VERDICTS = {
  create: { icon: FolderPlus, tone: "text-foreground" },
  "create-in-used": { icon: TriangleAlert, tone: "text-foreground" },
  adopt: { icon: FolderCheck, tone: "text-foreground" },
  blocked: { icon: CircleAlert, tone: "text-destructive-ink" },
};

function formatDate(value) {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat(undefined, { dateStyle: "long" }).format(date);
}

function verdictText(report) {
  if (!report) return null;
  if (report.action === "adopt") {
    const created = formatDate(report.manifest?.createdAt);
    return {
      title: "An existing Inertia workspace was found here.",
      body: created
        ? `It will be opened as it is - nothing gets overwritten. Created ${created}.`
        : "It will be opened as it is - nothing gets overwritten.",
    };
  }
  if (report.action === "create-in-used") {
    const count = report.entryCount ?? 0;
    return {
      title: `This folder already has ${count} ${count === 1 ? "item" : "items"} in it.`,
      body: "Inertia will add its own folders alongside them.",
    };
  }
  if (report.action === "blocked") {
    return { title: report.error || "This folder cannot be used.", body: null };
  }
  return { title: "A new workspace will be created here.", body: null };
}

export function SetupView() {
  const { phase, status, error, native, client, browse, inspect, configure } = useWorkspace();

  const [path, setPath] = React.useState("");
  const [touched, setTouched] = React.useState(false);
  const [report, setReport] = React.useState(null);
  const [checking, setChecking] = React.useState(false);
  const [saving, setSaving] = React.useState(false);
  const [failure, setFailure] = React.useState(null);
  const [showLayout, setShowLayout] = React.useState(false);
  const [directories, setDirectories] = React.useState([]);

  // The suggestion arrives with the status, which lands after the first paint.
  // Adopting it only while the field is untouched keeps a late status from
  // stamping over something the user has already typed.
  React.useEffect(() => {
    if (touched || !status) return;
    const initial = status.suggested || status.root || "";
    if (initial) setPath(initial);
  }, [status, touched]);

  // The layout is the backend's list, fetched rather than mirrored, so the
  // disclosure can never drift from what actually gets created on disk.
  React.useEffect(() => {
    let alive = true;
    Promise.resolve(client.layout?.())
      .then((result) => {
        if (alive && Array.isArray(result?.directories)) setDirectories(result.directories);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [client]);

  // Every keystroke on a path field would otherwise be a disk stat. The trailing
  // edge is the only one worth answering - a half-typed path has no useful
  // verdict, and showing one would make the panel flicker between outcomes.
  React.useEffect(() => {
    const candidate = path.trim();
    if (!candidate) {
      setReport(null);
      setChecking(false);
      return undefined;
    }
    let alive = true;
    setChecking(true);
    const timer = window.setTimeout(() => {
      Promise.resolve(inspect(candidate))
        .then((result) => {
          if (alive) setReport(result);
        })
        .catch((problem) => {
          if (alive) setReport({ path: candidate, action: "blocked", error: problem.message });
        })
        .finally(() => {
          if (alive) setChecking(false);
        });
    }, DEBOUNCE_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [path, inspect]);

  async function pick() {
    const chosen = await browse();
    if (!chosen) return;
    setTouched(true);
    setPath(chosen);
  }

  async function confirm() {
    const candidate = path.trim();
    if (!candidate) return;
    setSaving(true);
    setFailure(null);
    try {
      await configure(candidate);
    } catch (problem) {
      setFailure(problem.message);
    } finally {
      setSaving(false);
    }
  }

  const verdict = verdictText(report);
  const Verdict = report ? (VERDICTS[report.action] ?? VERDICTS.create).icon : null;
  const tone = report ? (VERDICTS[report.action] ?? VERDICTS.create).tone : "";
  const blocked = report?.action === "blocked";
  const adopting = report?.action === "adopt";
  const topLevel = directories.filter((entry) => !entry.path.includes("/"));

  // Three ways to arrive here with something already broken: the status call
  // failed, the remembered folder is gone, or it is no longer a workspace. They
  // all end in the same place - pick a folder - so they share one banner.
  let recovery = null;
  if (phase === "error") {
    recovery = { title: "Inertia could not read its workspace.", body: error };
  } else if (status?.missing) {
    recovery = {
      title: "The workspace folder is missing.",
      body: status.root
        ? `Nothing is at ${status.root} any more. Point Inertia at it again, or choose another folder.`
        : "Point Inertia at the folder again, or choose another one.",
    };
  } else if (status?.stale) {
    recovery = {
      title: "That folder is no longer an Inertia workspace.",
      body: status.root
        ? `${status.root} exists, but its manifest is gone. Choose a folder to use instead.`
        : "Its manifest is gone. Choose a folder to use instead.",
    };
  }

  return (
    <ScrollArea fade className="flex-1 bg-background">
      {/*
        Centred on both axes, and `min-h-full` is what makes the vertical half
        work: a flex child centres against its parent's height, and inside a
        scroll area that height is the content's own unless it is told to fill.
        `justify-center` with `py-16` rather than a fixed offset so a tall
        recovery notice pushes the block apart instead of off the top - the
        padding becomes a floor once the content outgrows the window.
      */}
      <div className="flex min-h-full w-full items-center justify-center px-8 py-16">
        <div className="flex w-full max-w-[38rem] flex-col">
          {/*
            Relative, not "/assets/...". Relative resolves the same under every
            origin this page has been served from: the dev server, Tauri's
            asset protocol, and the file:// URL the earlier Electron build
            loaded - where a leading slash meant the root of the drive, and
            this exact image resolved to file:///D:/assets/logo-256.png on the
            first screen a new user ever saw. `icons.test.js` keeps it that way.
          */}
          <img
            src="./assets/logo-256.png"
            alt=""
            className="size-9 shrink-0 rounded-lg object-contain"
          />

          <h1 className="mt-6 text-xl font-medium text-foreground">
            {recovery ? "Point Inertia at a workspace" : "Choose a workspace folder"}
          </h1>
          <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
            Everything Inertia keeps lives in one folder: your settings, agents, skills, plugins,
            conversations, memory and keys. Nothing is stored anywhere else.
          </p>
          <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
            That makes it portable. Copy the folder to another machine, point a fresh install at it,
            and everything is back exactly as you left it.
          </p>

          {recovery ? (
            <div className="mt-6 animate-slide-up rounded-xl fill-whisper px-3.5 py-2.5">
              <div className="flex items-start gap-2.5">
                <TriangleAlert
                  className="size-4 shrink-0 translate-y-px text-foreground"
                  aria-hidden="true"
                />
                <div className="min-w-0">
                  <p className="text-[13px] text-foreground">{recovery.title}</p>
                  {recovery.body ? (
                    <p className="mt-1 break-words text-xs leading-relaxed text-muted-foreground">
                      {recovery.body}
                    </p>
                  ) : null}
                </div>
              </div>
            </div>
          ) : null}

          <div className="mt-8 flex flex-col gap-2">
            <label htmlFor="workspace-path" className="text-xs font-medium text-muted-foreground">
              Folder
            </label>
            <div className="flex items-center gap-2">
              <Input
                id="workspace-path"
                size="sm"
                className="flex-1 font-mono text-[12px]"
                spellCheck={false}
                autoComplete="off"
                placeholder="C:\Users\you\Documents\Inertia"
                value={path}
                onChange={(event) => {
                  setTouched(true);
                  setFailure(null);
                  setPath(event.target.value);
                }}
              />
              {native ? (
                // Sized and shaped to the field beside it rather than to the
                // button scale. `size="sm"` is `h-[1.875rem]` and a pill, while
                // `Input size="sm"` is `h-8` and `rounded-md` - so the default
                // pairing is two pixels short and a different shape, which in a
                // single row reads as a mistake. Overridden here rather than in
                // the primitive, because every other button in the app is a
                // pill on purpose and this one is only matching its neighbour.
                <Button
                  variant="subtle"
                  size="sm"
                  className="h-8 rounded-md"
                  onClick={pick}
                  disabled={saving}
                >
                  <FolderOpen aria-hidden="true" />
                  Browse
                </Button>
              ) : null}
            </div>
            {native ? null : (
              <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
                Running outside the desktop app, so there is no folder picker here. Type the full
                path instead.
              </p>
            )}
          </div>

          {/* Sized by its contents, never by a floor. Every state here is one
              line except the two-line verdicts, and a minimum height tuned for
              the tallest of them left the common case sitting in a box with
              nothing in the bottom third of it. The icon aligns to the first
              line rather than the block, so a one-line row reads as centred and
              a two-line one still hangs off its title. */}
          <div className="mt-3 rounded-xl fill-whisper px-3.5 py-2.5">
            {/* Keyed on the outcome, so each verdict fades in as a new thing
                rather than the previous sentence being edited into it. */}
            <div
              key={checking ? "checking" : (report?.action ?? "empty")}
              className="animate-fade-in"
            >
              {checking ? (
                <div className="flex min-h-5 items-center gap-2.5 text-[13px] text-muted-foreground">
                  <Spinner size="sm" label="Checking the folder" />
                  Checking that folder…
                </div>
              ) : verdict ? (
                <div className="flex items-start gap-2.5">
                  <Verdict
                    className={cn("size-4 shrink-0 translate-y-px", tone)}
                    aria-hidden="true"
                  />
                  <div className="min-w-0">
                    <p className={cn("text-[13px] leading-5", blocked ? tone : "text-foreground")}>
                      {verdict.title}
                    </p>
                    {verdict.body ? (
                      <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
                        {verdict.body}
                      </p>
                    ) : null}
                  </div>
                </div>
              ) : (
                <p className="min-h-5 text-[13px] leading-5 text-muted-foreground">
                  Enter a folder to see what Inertia will do with it.
                </p>
              )}
            </div>
          </div>

          {topLevel.length ? (
            <div className="mt-4">
              <Button
                variant="ghost"
                size="sm"
                // Pulled back by exactly its own padding (`px-3.5`), so the
                // chevron sits flush with the left edge of the field and the
                // panels above it. `-ml-3` left it two pixels proud of them,
                // which is enough to see in a column this narrow.
                className="-ml-3.5 text-muted-foreground"
                aria-expanded={showLayout}
                onClick={() => setShowLayout((open) => !open)}
              >
                <ChevronRight
                  aria-hidden="true"
                  className={cn(
                    "transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
                    showLayout && "rotate-90"
                  )}
                />
                What goes in this folder
              </Button>
              <Collapse open={showLayout}>
                <div className="mt-2 flex flex-col gap-2.5 rounded-xl fill-whisper px-3.5 py-3">
                  {topLevel.map((entry) => (
                    <div key={entry.path} className="flex items-start gap-2.5">
                      <FolderTree
                        className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
                        aria-hidden="true"
                      />
                      <div className="min-w-0">
                        <p className="text-xs text-foreground">
                          {entry.label}
                          <span className="ml-1.5 font-mono text-[0.6875rem] text-muted-foreground">
                            {entry.path}/
                          </span>
                        </p>
                        <p className="mt-0.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
                          {entry.description}
                        </p>
                      </div>
                    </div>
                  ))}
                </div>
              </Collapse>
            </div>
          ) : null}

          {failure ? (
            <p className="mt-4 animate-fade-in text-[13px] leading-relaxed text-destructive-ink">
              {failure}
            </p>
          ) : null}

          <div className="mt-8 flex items-center gap-3">
            <Button
              variant="primary"
              size="md"
              onClick={confirm}
              disabled={saving || blocked || !path.trim()}
            >
              {saving ? <Spinner size="sm" /> : <FolderOpen aria-hidden="true" />}
              {adopting ? "Open this workspace" : "Create workspace"}
            </Button>
            <span className="text-xs text-muted-foreground">
              You can move the folder later and point Inertia at it again.
            </span>
          </div>
        </div>
      </div>
    </ScrollArea>
  );
}
