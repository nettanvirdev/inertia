import * as React from "react";
import {
  Blocks,
  CircleAlert,
  ExternalLink,
  KeyRound,
  ListChecks,
  PowerOff,
  Trash2,
  RotateCw,
  Search,
  SearchX,
  X,
} from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { composio } from "@/lib/integrations";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Spinner } from "@/components/ui/spinner";
import { Checkbox } from "@/components/ui/checkbox";
import { ScrollArea } from "@/components/ui/scroll-area";
import { EmptyState } from "@/components/ui/empty-state";
import { IconButton } from "@/components/ui/icon-button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { GUTTER } from "@/components/layout/View";
import { AppLogo } from "../AppLogo";
import { Section } from "../fields";

/**
 * Composio: hosted apps, connected by OAuth, offering their operations as tools.
 *
 * The screen is the flow. An app is connected by picking it out of the live
 * catalogue, finishing an OAuth handshake in a browser window, and then choosing
 * which of its operations an agent may actually call. None of that is a record
 * the user edits by hand, which is why this tab is its own screen rather than
 * the generic folder manager the Skills tab uses.
 */

/** Composio finishes a handshake in seconds; a person takes rather longer. */
const POLL_EVERY_MS = 2500;
const POLL_TIMEOUT_MS = 180000;

/**
 * What each status means, and what the user is meant to do about it.
 *
 * The `note` is the part that was missing. A row reading "Expired" beside a
 * Reconnect button is only obvious to somebody who already knows that no tool
 * from this app has worked since the token lapsed - and until the status was
 * synced at all, the row said "Connected" while every call failed inside an
 * agent turn with the vendor's own opaque wording.
 *
 * MISSING is ours, not Composio's: it means the project has no account under
 * the id this record names, which is what a connection revoked from Composio's
 * own dashboard looks like from here.
 */
const STATUS = {
  INITIATED: { label: "Waiting", variant: "warning", note: "Finish the sign-in in the popup." },
  ACTIVE: { label: "Connected", variant: "success", note: "" },
  FAILED: {
    label: "Failed",
    variant: "danger",
    note: "The sign-in did not complete, so no tool from this app will work. Reconnect to try again.",
  },
  EXPIRED: {
    label: "Expired",
    variant: "warning",
    note: "The credential has lapsed. Every tool from this app is failing until you reconnect.",
  },
  MISSING: {
    label: "Gone",
    variant: "danger",
    note: "Composio no longer has this account, so it was probably revoked there. Reconnect to make a new one.",
  },
};

function statusOf(record) {
  const raw = String(record?.status ?? "").toUpperCase();
  return STATUS[raw] ?? { label: raw || "Unknown", variant: "neutral", note: "" };
}

/**
 * The OAuth window.
 *
 * Opened from inside the user's own click handler, always. A `window.open` that
 * happens after an await has lost the user activation that permits it, and the
 * popup is silently blocked - so the blank window is claimed first and pointed
 * at the URL once Composio has answered.
 */
function openAuthWindow(url) {
  try {
    return window.open(url ?? "", "inertia-composio", "width=600,height=760");
  } catch {
    return null;
  }
}

/** A link out of the app. The backend opens it in the system browser so the
 *  window itself never navigates away from the app it is. */
function openExternal(url) {
  if (!url) return;
  if (window.electronAPI?.openExternal) window.electronAPI.openExternal(url);
  else window.open(url, "_blank", "noopener,noreferrer");
}

/**
 * How old the catalogue on screen is, in words a glance can read.
 *
 * A timestamp would be more precise and less useful: the question the user is
 * actually asking is "is this from before or after I changed something at
 * Composio", and an answer in elapsed time answers it without arithmetic.
 */
function ageLabel(at) {
  if (!at) return "";
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 45) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.round(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}

/* -- connected apps --------------------------------------------------------- */

function ConnectionCard({ record, busy, onManage, onReconnect, onDisconnect }) {
  const status = statusOf(record);

  return (
    <div className="flex animate-slide-up items-center gap-3 rounded-2xl fill-control px-3 py-2.5">
      <AppLogo src={record.logo} name={record.name || record.toolkitSlug} className="size-8" />

      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-[13px] text-foreground">
            {record.name || record.toolkitSlug}
          </span>
          <Badge size="sm" variant={status.variant}>
            {status.label}
          </Badge>
        </div>
        <p className="truncate text-[11px] text-muted-foreground">
          {[record.label || "No account name reported", record.toolkitSlug]
            .filter(Boolean)
            .join(" · ")}
        </p>
        {status.note && status.variant !== "success" ? (
          <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{status.note}</p>
        ) : null}
      </div>

      <Button variant="subtle" size="xs" onClick={onManage}>
        <ListChecks />
        Manage tools
      </Button>
      <Button variant="subtle" size="xs" disabled={busy} onClick={onReconnect}>
        <RotateCw />
        Reconnect
      </Button>
      {/* In words, not an icon alone. A power symbol at the end of the row
          was the only way to be rid of a connection, and nobody found it -
          least of all for a row stuck at INITIALIZING or EXPIRED, where
          "disconnect" is not even the right word: there is nothing
          connected, there is a record to remove. */}
      <Button variant="subtle" size="xs" disabled={busy} onClick={onDisconnect}>
        {status.variant === "success" ? <PowerOff /> : <Trash2 />}
        {status.variant === "success" ? "Disconnect" : "Remove"}
      </Button>
    </div>
  );
}

/** The live handshake: what the user is waiting on, and how to get back to it. */
function PendingConnection({ pending, onReopen, onDismiss }) {
  if (!pending) return null;

  const failed = pending.phase === "failed" || pending.phase === "timeout";

  return (
    <div className="flex animate-slide-up items-start gap-3 rounded-2xl fill-whisper px-3 py-2.5">
      {failed ? (
        <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive-ink" aria-hidden="true" />
      ) : (
        <Spinner size="sm" className="mt-0.5 shrink-0" />
      )}
      <div className="min-w-0 flex-1">
        <p className="text-[13px] text-foreground">
          {pending.phase === "starting"
            ? `Asking Composio to set up ${pending.name}`
            : failed
              ? `${pending.name} was not connected`
              : `Waiting for you to finish ${pending.name} in the browser`}
        </p>
        <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
          {failed
            ? (pending.error ??
              "The handshake did not finish. Nothing was connected, so it is safe to try again.")
            : "The sign-in happens in a separate window. It is easy to lose behind the app, so reopen it if it has gone missing."}
        </p>
      </div>
      {!failed && pending.redirectUrl ? (
        <Button variant="subtle" size="xs" onClick={onReopen}>
          <ExternalLink />
          Reopen the window
        </Button>
      ) : null}
      <IconButton size="lg" label="Dismiss" onClick={onDismiss}>
        <X />
      </IconButton>
    </div>
  );
}

/* -- browse ----------------------------------------------------------------- */

/**
 * One app in the catalogue.
 *
 * Memoised, and skipped while off screen. The catalogue is several hundred
 * tiles, each with a logo of its own, and drawing all of them on every keystroke
 * of the search box - or repainting all of them on every scroll frame - is what
 * made this screen feel like it was catching on something. `content-visibility`
 * lets the browser skip the layout and paint of a tile nobody can see, and the
 * intrinsic size keeps the scrollbar honest while it does; `auto` there means
 * the real height is remembered once a tile has been drawn.
 */
const ToolkitTile = React.memo(function ToolkitTile({ toolkit, connected, onOpen }) {
  return (
    <button
      type="button"
      onClick={() => onOpen(toolkit)}
      className={cn(
        "flex items-center gap-2.5 rounded-2xl fill-control px-3 py-2.5 text-left outline-none",
        "transition-colors duration-150 ease-out hover:fill-control-hover",
        "focus-visible:fill-control-hover",
        "[content-visibility:auto] [contain-intrinsic-size:auto_54px]"
      )}
    >
      <AppLogo src={toolkit.logo} name={toolkit.name} className="size-7" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] text-foreground">{toolkit.name}</span>
        <span className="block truncate text-[11px] text-muted-foreground">
          {toolkit.description || toolkit.slug}
        </span>
      </span>
      {connected ? (
        <Badge size="sm" variant="success">
          Connected
        </Badge>
      ) : null}
    </button>
  );
});

/**
 * An app Composio carries that Inertia cannot start a sign-in for.
 *
 * Shown rather than hidden, because an app missing from the list is
 * indistinguishable from an app Composio does not have, and the user goes off
 * looking for it somewhere else. Quieter than a tile in every way that costs
 * nothing to read - no plate, no logo round trip, no hover state - because this
 * is a section people scan once and then stop seeing.
 */
const BlockedToolkitRow = React.memo(function BlockedToolkitRow({ toolkit }) {
  return (
    <div className="flex items-start gap-2 rounded-xl px-2.5 py-2 [content-visibility:auto] [contain-intrinsic-size:auto_48px]">
      <div className="min-w-0 flex-1">
        <p className="truncate text-[12px] text-muted-foreground">{toolkit.name}</p>
        <p className="text-[11px] leading-relaxed text-muted-foreground/70">{toolkit.reason}</p>
      </div>
      <Button variant="ghost" size="xs" onClick={() => openExternal(toolkit.setupUrl)}>
        <ExternalLink />
        Set up
      </Button>
    </div>
  );
});

/**
 * One toolkit, in full, with the button that starts the handshake.
 *
 * A centred dialog rather than an edge drawer, and that is not only taste. The
 * window has rounded corners; a drawer pinned to `inset-y-0 right-0` has square
 * ones, and its edge lands exactly on the two corners the window spent effort
 * rounding. More to the point, this is a decision - connect this app or do not
 * - and a decision belongs in front of the thing it is about, not beside it.
 */
function ToolkitDialog({ toolkit, connected, busy, onClose, onConnect }) {
  return (
    <Dialog
      open={Boolean(toolkit)}
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      ariaLabel={toolkit ? `${toolkit.name} details` : "App details"}
    >
      {toolkit ? (
        <>
          <div className="flex items-start gap-3 pr-8">
            <AppLogo src={toolkit.logo} name={toolkit.name} className="size-10" />
            <div className="min-w-0 flex-1">
              <DialogTitle className="truncate">{toolkit.name}</DialogTitle>
              <p className="truncate font-mono text-[11px] text-muted-foreground">{toolkit.slug}</p>
            </div>
          </div>

          <DialogBody className="mt-4 flex flex-col gap-3">
            <DialogDescription className="text-[13px] leading-relaxed">
              {toolkit.description || "Composio publishes no description for this app."}
            </DialogDescription>

            {toolkit.categories?.length ? (
              <div className="flex flex-wrap gap-1.5">
                {toolkit.categories.map((category) => (
                  <Badge key={category} size="sm">
                    {category}
                  </Badge>
                ))}
              </div>
            ) : null}

            <p className="rounded-xl fill-whisper px-3 py-2.5 text-[11px] leading-relaxed text-muted-foreground">
              Connecting opens {toolkit.name} in a browser window and asks you to sign in there.
              Composio holds the credentials afterwards; Inertia only ever holds the connection.
            </p>
          </DialogBody>

          <DialogFooter className="mt-4">
            <Button
              variant="primary"
              size="sm"
              disabled={busy}
              onClick={() => onConnect(toolkit)}
              className="flex-1"
            >
              {busy ? <Spinner size="sm" /> : null}
              {connected ? `Connect another ${toolkit.name} account` : `Connect ${toolkit.name}`}
            </Button>
          </DialogFooter>
        </>
      ) : null}
    </Dialog>
  );
}

/* -- manage tools ----------------------------------------------------------- */

/**
 * Which of an app's operations an agent may call.
 *
 * Nothing saved yet means everything is selected, because that is what the
 * backend does with an empty list and a dialog that showed nothing ticked would
 * be describing a state the app is not in.
 */
function ManageToolsDialog({ connection, open, onClose, onSaved }) {
  const { toast } = useToast();
  const [tools, setTools] = React.useState([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState(null);
  const [query, setQuery] = React.useState("");
  const [enabled, setEnabled] = React.useState(true);
  const [chosen, setChosen] = React.useState(() => new Set());
  const [saving, setSaving] = React.useState(false);
  // Bumped to re-run the read below. The tool list is cached in the backend
  // for the life of the process, which is right for a catalogue that changes when
  // Composio ships and wrong on the afternoon somebody adds a tool to a toolkit
  // and cannot work out why Inertia will not offer it. Before this the only
  // remedy was to quit the app, and nothing on screen said so.
  const [reload, setReload] = React.useState(0);

  React.useEffect(() => {
    if (!open || !connection) return undefined;
    let alive = true;
    setLoading(true);
    setError(null);
    setEnabled(connection.enabled !== false);

    composio
      .tools(connection.toolkitSlug)
      .then((rows) => {
        if (!alive) return;
        setTools(rows);
        const saved = connection.enabledTools ?? [];
        setChosen(new Set(saved.length ? saved : rows.map((tool) => tool.slug)));
      })
      .catch((failure) => alive && setError(failure.message))
      .finally(() => alive && setLoading(false));

    return () => {
      alive = false;
    };
  }, [open, connection, reload]);

  // The filter is cleared when the dialog opens, not when the list is re-read:
  // a user who pressed Refresh was looking at something and should still be
  // looking at it afterwards.
  React.useEffect(() => {
    if (open) setQuery("");
  }, [open]);

  async function refreshTools() {
    if (!connection) return;
    setLoading(true);
    try {
      await composio.refresh(connection.toolkitSlug);
    } catch {
      // A refresh that could not drop the cache still re-reads below, which is
      // the part the user asked for.
    }
    setReload((n) => n + 1);
  }

  const filtered = React.useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return tools;
    return tools.filter((tool) =>
      `${tool.name} ${tool.slug} ${tool.description}`.toLowerCase().includes(needle)
    );
  }, [tools, query]);

  function toggle(slug) {
    setChosen((prev) => {
      const next = new Set(prev);
      if (next.has(slug)) next.delete(slug);
      else next.add(slug);
      return next;
    });
  }

  async function save() {
    if (!connection) return;
    setSaving(true);
    try {
      // An empty list means "everything" to the backend, so saving one would do
      // the opposite of what the user just asked for. Turning the app off is
      // what they actually meant, so that is what is saved.
      const none = chosen.size === 0;
      await composio.setPermissions(connection.id, {
        enabled: none ? false : enabled,
        enabledTools: none ? [] : [...chosen],
      });
      toast({
        variant: "success",
        title: `${connection.name || connection.toolkitSlug} updated`,
        description: none
          ? "No tools were left selected, so the app is switched off rather than offering all of them."
          : `${chosen.size} of ${tools.length} tools are offered to your agents.`,
      });
      onSaved?.();
      onClose();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()} size="lg">
      <DialogTitle>
        {connection ? `${connection.name || connection.toolkitSlug} tools` : "Tools"}
      </DialogTitle>
      <DialogDescription>
        Every tool ticked here is described to the model on every turn, so a shorter list is a
        sharper agent as well as a cheaper one.
      </DialogDescription>

      <DialogBody className="mt-4 flex flex-col gap-3">
        <div className="flex items-center gap-3 rounded-xl fill-whisper px-3 py-2.5">
          <div className="min-w-0 flex-1">
            <p className="text-[13px] text-foreground">Offer this app to agents</p>
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              Off keeps the connection and stops every tool it provides.
            </p>
          </div>
          <Switch checked={enabled} onCheckedChange={setEnabled} label="Offer this app to agents" />
        </div>

        <div className="flex items-center gap-2">
          <Input
            size="sm"
            className="flex-1"
            value={query}
            leadingIcon={<Search />}
            placeholder="Filter tools"
            aria-label="Filter tools"
            onChange={(e) => setQuery(e.target.value)}
          />
          <Button
            variant="subtle"
            size="xs"
            onClick={() => setChosen(new Set(tools.map((tool) => tool.slug)))}
          >
            Select all
          </Button>
          <Button variant="subtle" size="xs" onClick={() => setChosen(new Set())}>
            Clear
          </Button>
          <Button variant="subtle" size="xs" disabled={loading} onClick={refreshTools}>
            <RotateCw />
            Refresh
          </Button>
        </div>

        {loading ? (
          <div className="flex items-center gap-2 py-6 text-[13px] text-muted-foreground">
            <Spinner size="sm" />
            Reading the tool list from Composio
          </div>
        ) : error ? (
          <p className="animate-fade-in text-[11px] leading-relaxed text-destructive-ink">
            {error}
          </p>
        ) : (
          <ScrollArea className="max-h-[min(24rem,50dvh)] pr-1" fade>
            <div className="flex animate-fade-in flex-col gap-0.5">
              {filtered.map((tool) => (
                // The row is the hit target, the checkbox is the tab stop. The
                // checkbox stops the click it already handled, or the row would
                // toggle it straight back.
                <div
                  key={tool.slug}
                  onClick={() => toggle(tool.slug)}
                  className={cn(
                    "flex cursor-pointer items-start gap-2.5 rounded-xl px-2.5 py-2",
                    "transition-colors duration-150 ease-out hover:fill-control-hover"
                  )}
                >
                  <span onClick={(e) => e.stopPropagation()}>
                    <Checkbox
                      size="sm"
                      className="mt-0.5"
                      checked={chosen.has(tool.slug)}
                      onCheckedChange={() => toggle(tool.slug)}
                      label={tool.name || tool.slug}
                    />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px] text-foreground">
                      {tool.name || tool.slug}
                    </span>
                    <span className="block text-[11px] leading-relaxed text-muted-foreground">
                      {tool.description || "Composio publishes no description for this tool."}
                    </span>
                  </span>
                </div>
              ))}
              {filtered.length ? null : (
                <p className="px-2.5 py-6 text-[13px] text-muted-foreground">
                  Nothing matches that filter.
                </p>
              )}
            </div>
          </ScrollArea>
        )}
      </DialogBody>

      <DialogFooter>
        <Button variant="secondary" size="sm" onClick={onClose} disabled={saving}>
          Cancel
        </Button>
        <Button variant="primary" size="sm" onClick={save} disabled={saving || loading}>
          {saving ? <Spinner size="sm" /> : null}
          Save
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

/* -- the screen ------------------------------------------------------------- */

/** The tile arrangement the header's grid/list toggle asks for. The other
 *  tabs read it through PluginManager; this screen draws its own tiles, so
 *  it has to read it itself - and for a long time did not, which made the
 *  toggle a control that did nothing on the one tab with the most tiles. */
function tilesClass(layout) {
  return layout === "rows" ? "flex flex-col gap-2" : "grid gap-2 md:grid-cols-2 2xl:grid-cols-3";
}

function ComposioScreen({ query, layout }) {
  const { toast } = useToast();
  const { openSettings } = useApp();

  // Read once, on the first render, so a remount that has a cached catalogue
  // never renders a loading state at all. This is the whole point of the cache:
  // switching tabs and coming back should look like coming back.
  const cached = composio.cachedToolkits();
  const cachedRows = composio.cachedConnections();

  const [configured, setConfigured] = React.useState(cached ? true : null);
  const [connections, setConnections] = React.useState(cachedRows ?? []);
  const [toolkits, setToolkits] = React.useState(cached?.items ?? []);
  const [fetchedAt, setFetchedAt] = React.useState(cached?.fetchedAt ?? 0);
  const [loading, setLoading] = React.useState(!cached);
  // The catalogue is the slow half - several hundred apps over paged requests
  // - and it is the half nobody came for. Its own flag, so the connected apps
  // draw as soon as they are read and Browse fills in underneath them.
  const [browsing, setBrowsing] = React.useState(!cached);
  const [refreshing, setRefreshing] = React.useState(false);
  const [error, setError] = React.useState(null);

  const [pending, setPending] = React.useState(null);
  const [detail, setDetail] = React.useState(null);
  const [managing, setManaging] = React.useState(null);
  const [disconnecting, setDisconnecting] = React.useState(null);

  const reloadConnections = React.useCallback(async () => {
    setConnections(await composio.connections());
  }, []);

  const alive = React.useRef(true);
  React.useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  // The app saying a connection or the catalogue changed - from a finished
  // handshake, another window, or an agent connecting an app mid-turn. Only
  // the stored rows are re-read: the catalogue is a file and a refresh of it
  // is something a person asks for.
  React.useEffect(
    () =>
      composio.onEvent((event) => {
        if (!alive.current) return;
        if (event?.type === "connection" || event?.type === "refresh") {
          reloadConnections().catch(() => {});
        }
      }),
    [reloadConnections]
  );

  /**
   * Read the connections and the catalogue.
   *
   * `refresh` is the button, and it is the only thing that goes past the cache.
   * Without it this ran on every mount, which is every tab switch and every
   * navigation, and the user watched the same spinner for the same list they
   * had just been reading. The catalogue is fetched whole and filtered in the
   * renderer for the same reason it always was: a round trip per keystroke
   * would make the search feel worse than a list that is already here.
   */
  const load = React.useCallback(async ({ refresh = false } = {}) => {
    if (refresh) setRefreshing(true);
    try {
      const ready = await composio.configured();
      if (!alive.current) return;
      setConfigured(ready);
      if (!ready) return;

      // The stored rows first, and on their own. They are files: they are the
      // list, they are already right, and they are what the user came to see.
      // Waiting for Composio to confirm them before drawing anything is what
      // made this screen blink its connected apps away on every visit.
      const stored = await composio.connections();
      if (!alive.current) return;
      setConnections(stored);
      setError(null);
      // Here, not in the `finally`. The screen used to say it drew the stored
      // rows first and then waited for the catalogue before drawing anything,
      // which is the same thing as not drawing them first.
      setLoading(false);

      setBrowsing(true);
      const catalogue = await composio.toolkits({ refresh });
      if (!alive.current) return;
      setToolkits(catalogue.items ?? []);
      setFetchedAt(catalogue.fetchedAt ?? Date.now());
    } catch (failure) {
      if (alive.current) setError(failure.message);
    } finally {
      if (alive.current) {
        setLoading(false);
        setBrowsing(false);
        setRefreshing(false);
      }
    }
  }, []);

  /**
   * Ask Composio whether the stored rows are still true, without holding the
   * screen up for the answer.
   *
   * A lapsed OAuth token says nothing when it lapses: no callback arrives, the
   * record still reads ACTIVE, and the first anyone hears of it is a tool
   * failing mid-turn. This is the screen finding out before an agent does - and
   * because it runs behind the list rather than in front of it, a slow or
   * missing network costs the freshness and never the list.
   */
  const verify = React.useCallback(async () => {
    const rows = await composio.sync().catch(() => null);
    if (alive.current && Array.isArray(rows)) setConnections(rows);
  }, []);

  // Connections are workspace records that change under this screen - another
  // window, a finished handshake - so they are always read on mount. Both reads
  // inside are local now: the records are files and the catalogue is a file
  // until someone refreshes it. Checking them against Composio happens after,
  // in its own time.
  React.useEffect(() => {
    load().then(verify);
  }, [load, verify]);

  /**
   * Poll until Composio says the handshake landed.
   *
   * The interval is cleared on unmount and on every phase change, because a
   * timer still polling a connection nobody is looking at writes status back to
   * a record for no reason and keeps the window from ever being idle.
   */
  React.useEffect(() => {
    if (!pending || pending.phase !== "waiting") return undefined;

    const deadline = Date.now() + POLL_TIMEOUT_MS;
    let alive = true;

    const timer = setInterval(async () => {
      if (!alive) return;
      if (Date.now() > deadline) {
        setPending((current) =>
          current && current.id === pending.id
            ? {
                ...current,
                phase: "timeout",
                error:
                  "Composio has not reported this as connected for three minutes. The connection is still on file, so try Reconnect once the sign-in is finished.",
              }
            : current
        );
        return;
      }

      try {
        const record = await composio.status(pending.id);
        if (!alive) return;
        const status = String(record?.status ?? "").toUpperCase();
        if (status === "ACTIVE") {
          setPending(null);
          // Replacing an older connection is only safe once the new one works.
          if (pending.replacingId) {
            try {
              await composio.disconnect(pending.replacingId);
            } catch {
              /* the new connection stands either way */
            }
          }
          await reloadConnections();
          toast({ variant: "success", title: `${pending.name} connected` });
        } else if (status === "FAILED" || status === "EXPIRED") {
          setPending((current) =>
            current && current.id === pending.id
              ? {
                  ...current,
                  phase: "failed",
                  error: `Composio reported the connection as ${status}.`,
                }
              : current
          );
          await reloadConnections();
        }
      } catch {
        // A single failed poll is a network blip, not a failed connection. The
        // deadline above is what ends this, not one bad answer.
      }
    }, POLL_EVERY_MS);

    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [pending, reloadConnections, toast]);

  /**
   * Start a handshake. Must be called straight from a click: the blank window
   * is claimed synchronously and pointed at the URL once Composio answers.
   */
  async function beginConnect(toolkit, replacingId = null) {
    const popup = openAuthWindow();
    setDetail(null);
    setPending({ id: null, name: toolkit.name, phase: "starting", redirectUrl: null, replacingId });

    try {
      const started = await composio.connect(toolkit.slug);
      if (popup && started.redirectUrl) popup.location.href = started.redirectUrl;
      else if (started.redirectUrl) openAuthWindow(started.redirectUrl);

      setPending({
        id: started.id,
        name: toolkit.name,
        phase: "waiting",
        redirectUrl: started.redirectUrl,
        replacingId,
      });
      await reloadConnections();
    } catch (failure) {
      popup?.close();
      setPending({
        id: null,
        name: toolkit.name,
        phase: "failed",
        error: failure.message,
        replacingId,
      });
    }
  }

  /**
   * Sign in again to a connection that already exists.
   *
   * Not the same thing as connecting afresh, and it used to be: the old path
   * made a second connection and deleted the first once the new one worked,
   * which threw away the list of operations the user had chosen for that app.
   * For a big toolkit that is real work, lost to a token expiring. This reuses
   * the record, so the only thing that changes is the credential behind it.
   *
   * Called straight from the click, like `beginConnect`, because the popup has
   * to be claimed before the first await or the browser blocks it.
   */
  async function beginReconnect(record) {
    const name = record.name || record.toolkitSlug;
    const popup = openAuthWindow();
    setPending({ id: null, name, phase: "starting", redirectUrl: null, replacingId: null });

    try {
      const started = await composio.reconnect(record.id);
      if (popup && started.redirectUrl) popup.location.href = started.redirectUrl;
      else if (started.redirectUrl) openAuthWindow(started.redirectUrl);

      setPending({
        id: started.id,
        name,
        phase: "waiting",
        redirectUrl: started.redirectUrl,
        // Nothing to replace: main reused the row and revoked the old account
        // itself, once the new link existed.
        replacingId: null,
      });
      await reloadConnections();
    } catch (failure) {
      popup?.close();
      setPending({ id: null, name, phase: "failed", error: failure.message, replacingId: null });
    }
  }

  async function confirmDisconnect() {
    const doomed = disconnecting;
    if (!doomed) return;
    try {
      await composio.disconnect(doomed.id);
      await reloadConnections();
      toast({ variant: "warning", title: `${doomed.name || doomed.toolkitSlug} disconnected` });
    } catch (failure) {
      toast({
        variant: "danger",
        title: "Could not disconnect that",
        description: failure.message,
      });
    } finally {
      setDisconnecting(null);
    }
  }

  const needle = query.trim().toLowerCase();

  const shownConnections = React.useMemo(() => {
    if (!needle) return connections;
    return connections.filter((row) =>
      `${row.name} ${row.toolkitSlug} ${row.label}`.toLowerCase().includes(needle)
    );
  }, [connections, needle]);

  const connectedSlugs = React.useMemo(
    () => new Set(connections.map((row) => row.toolkitSlug)),
    [connections]
  );

  /**
   * The catalogue split by whether Inertia can start a sign-in for the app.
   *
   * Derived from `toolkits` and nothing else, so a Refresh that reclassifies an
   * app - because the user has since created its auth config at Composio - moves
   * it between the two lists on the next render. Memoising the classification
   * itself, rather than the split of a freshly fetched list, is exactly how an
   * app would stay in the wrong section until restart.
   */
  const [connectable, blocked] = React.useMemo(() => {
    const matching = toolkits.filter(
      (toolkit) =>
        !needle ||
        `${toolkit.name} ${toolkit.slug} ${toolkit.description} ${(toolkit.categories ?? []).join(" ")}`
          .toLowerCase()
          .includes(needle)
    );
    const yes = [];
    const no = [];
    for (const toolkit of matching) (toolkit.connectable ? yes : no).push(toolkit);
    return [yes, no.sort((a, b) => a.name.localeCompare(b.name))];
  }, [toolkits, needle]);

  /** The connectable half, grouped by category. */
  const groups = React.useMemo(() => {
    const byCategory = new Map();
    for (const toolkit of connectable) {
      const category = toolkit.categories?.[0] || "Other";
      if (!byCategory.has(category)) byCategory.set(category, []);
      byCategory.get(category).push(toolkit);
    }
    return [...byCategory.entries()]
      .map(([name, items]) => ({ name, items: items.sort((a, b) => a.name.localeCompare(b.name)) }))
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [connectable]);

  if (loading) {
    return (
      <div className={cn("flex min-h-0 flex-1 items-start pt-6", GUTTER)}>
        <div className="flex items-center gap-2 text-[13px] text-muted-foreground">
          <Spinner size="sm" />
          Reading this workspace
        </div>
      </div>
    );
  }

  if (configured === false) {
    return (
      <div className={cn("min-h-0 flex-1 pt-2", GUTTER)}>
        <EmptyState
          icon={KeyRound}
          title="Composio is not set up yet"
          description="Composio is a hosted catalogue of app integrations: it holds the sign-in for your Gmail, Linear or Notion account and exposes each app's operations as tools. Inertia needs a Composio API key before it can show you any of that, and the key is stored as a workspace secret rather than in a settings file."
          action={
            <Button variant="primary" size="sm" onClick={() => openSettings("secrets")}>
              <KeyRound />
              Add the API key
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <>
      <ScrollArea className={cn("min-h-0 flex-1 pt-2 pb-8", GUTTER)}>
        {/* Mounted fresh on every tab switch (the view keys on the kind), so
            the arrival is the tab change. */}
        <div className="flex w-full animate-fade-in flex-col gap-8">
          <Section
            title="Connected apps"
            count={shownConnections.length}
            description="Read from this workspace's own files, so they are here the moment the screen is. Composio is asked afterwards whether they still work."
            action={
              <Button
                variant="subtle"
                size="xs"
                disabled={refreshing}
                onClick={() => load({ refresh: true }).then(verify)}
              >
                {refreshing ? <Spinner size="sm" /> : <RotateCw />}
                Refresh
              </Button>
            }
          >
            <PendingConnection
              pending={pending}
              onReopen={() => openAuthWindow(pending?.redirectUrl)}
              onDismiss={() => setPending(null)}
            />

            {error ? (
              <p className="animate-fade-in text-[11px] leading-relaxed text-destructive-ink">
                {error}
              </p>
            ) : null}

            {shownConnections.length ? (
              <div className="flex flex-col gap-1.5">
                {shownConnections.map((record) => (
                  <ConnectionCard
                    key={record.id}
                    record={record}
                    busy={Boolean(pending)}
                    onManage={() => setManaging(record)}
                    onReconnect={() => beginReconnect(record)}
                    onDisconnect={() => setDisconnecting(record)}
                  />
                ))}
              </div>
            ) : connections.length ? (
              <EmptyState
                icon={SearchX}
                title="Nothing matches"
                description={`None of your ${connections.length} connected apps match that search.`}
              />
            ) : (
              <EmptyState
                icon={Blocks}
                title="No apps connected"
                description="Pick one below and sign in to it. Its operations become tools every agent can call, and the credentials stay at Composio."
              />
            )}
          </Section>

          <Section
            title="Browse"
            count={connectable.length}
            description={
              fetchedAt
                ? `Apps you can connect from here, by signing in. The catalogue was read ${ageLabel(fetchedAt)} and is kept in the workspace, so opening this screen - or the app - shows it rather than fetching it again. Refresh reads it afresh.`
                : "Apps you can connect from here, by signing in."
            }
            action={
              <Button
                variant="subtle"
                size="xs"
                disabled={refreshing}
                onClick={() => load({ refresh: true }).then(verify)}
              >
                {refreshing ? <Spinner size="sm" /> : <RotateCw />}
                Refresh
              </Button>
            }
          >
            {browsing && !groups.length ? (
              <div className="flex items-center gap-2 py-6 text-[13px] text-muted-foreground">
                <Spinner size="sm" />
                Asking Composio what is available
              </div>
            ) : groups.length ? (
              <div className="flex flex-col gap-5">
                {groups.map((group) => (
                  <div key={group.name} className="flex flex-col gap-2">
                    <h3 className="text-[11px] font-medium text-muted-foreground">{group.name}</h3>
                    <div className={tilesClass(layout)}>
                      {group.items.map((toolkit) => (
                        <ToolkitTile
                          key={toolkit.slug}
                          toolkit={toolkit}
                          connected={connectedSlugs.has(toolkit.slug)}
                          onOpen={setDetail}
                        />
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <EmptyState
                icon={SearchX}
                title="No apps match"
                description="Nothing in the Composio catalogue matches that search."
              />
            )}
          </Section>

          {blocked.length ? (
            <Section
              title="Needs setting up at Composio first"
              count={blocked.length}
              description="Composio carries these, but signing in to them needs credentials of your own - an API key, or an OAuth client from the vendor's developer console. Save one as an auth config at Composio and press Refresh: the app moves up into Browse."
            >
              <div
                className={
                  layout === "rows"
                    ? "flex flex-col gap-0.5"
                    : "grid gap-0.5 md:grid-cols-2 2xl:grid-cols-3"
                }
              >
                {blocked.map((toolkit) => (
                  <BlockedToolkitRow key={toolkit.slug} toolkit={toolkit} />
                ))}
              </div>
            </Section>
          ) : null}
        </div>
      </ScrollArea>

      <ToolkitDialog
        toolkit={detail}
        connected={detail ? connectedSlugs.has(detail.slug) : false}
        busy={Boolean(pending) && pending.phase !== "failed" && pending.phase !== "timeout"}
        onClose={() => setDetail(null)}
        onConnect={(toolkit) => beginConnect(toolkit)}
      />

      <ManageToolsDialog
        connection={managing}
        open={Boolean(managing)}
        onClose={() => setManaging(null)}
        onSaved={reloadConnections}
      />

      <ConfirmDialog
        open={Boolean(disconnecting)}
        onOpenChange={(open) => !open && setDisconnecting(null)}
        destructive
        title={
          disconnecting
            ? `Disconnect ${disconnecting.name || disconnecting.toolkitSlug}?`
            : "Disconnect?"
        }
        description="Its tools stop being offered to every agent and the account is revoked at Composio. Connecting again means signing in again."
        confirmLabel="Disconnect"
        onConfirm={confirmDisconnect}
      />
    </>
  );
}

export const COMPOSIO_KIND = {
  id: "composio",
  collection: "plugins.composio",
  label: "Composio",
  icon: Blocks,
  noun: "connection",
  nounPlural: "connections",
  searchPlaceholder: "Search apps",
  addLabel: "Connect app",
  // An app is connected from the catalogue on the page, not from a dialog the
  // header could open.
  hideAdd: true,
  Screen: ComposioScreen,
};
