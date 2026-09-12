import { workspaceClient } from "./workspace-client";

/**
 * The renderer's view of the three tool bridges: Composio, MCP and OpenAPI.
 *
 * Same shape as `lib/llm.js` and for the same reason. One adapter forwards to
 * the preload bridge and unwraps the `{ ok, data }` envelope into a value or a
 * thrown Error, so no screen repeats that; a stand-in keeps the screens
 * renderable in a plain browser tab, which is exactly where this UI gets
 * iterated on.
 *
 * The stand-in never fakes a connection. A read degrades to an empty list, so a
 * screen draws its empty state instead of an error wall, and anything that
 * would change the world says plainly that it needs the desktop app.
 */

const MCP_COLLECTION = "plugins.mcp";
const OPENAPI_COLLECTION = "plugins.openapi";

function unwrap(reply) {
  if (!reply || typeof reply !== "object") throw new Error("The app did not answer.");
  if (reply.ok) return reply.data;
  throw new Error(reply.error || "That request was refused.");
}

function api(name) {
  return typeof window === "undefined" ? null : (window[name] ?? null);
}

function unavailable(what) {
  throw new Error(
    `${what} needs the desktop app. A browser tab has no main process to run the request in, and the credentials it needs are deliberately not reachable from the page.`
  );
}

/** True when all three bridges are present, which is only true inside Electron. */
export function isDesktop() {
  return Boolean(api("composioAPI") && api("mcpAPI") && api("openapiAPI"));
}

/**
 * Drop the main process's cached tool lists.
 *
 * Called after a write that went around a bridge handler (see `mcp.update`),
 * because the handler that was skipped is also the thing that would have
 * invalidated the cache, and a stale cache means an agent calling the old
 * config until the app restarts.
 */
async function invalidateTools() {
  try {
    await api("agentAPI")?.invalidate();
  } catch {
    /* a cache that will not drop is not worth failing a save over */
  }
}

/* -- Composio --------------------------------------------------------------- */

/**
 * How long a catalogue is worth showing without asking again.
 *
 * Composio's list of apps changes when Composio ships, which is not several
 * times an afternoon, so the cache is deliberately long-lived and Refresh is
 * how it is dropped. The ceiling exists only so an app left open for days
 * eventually notices the world moved, not as a refresh interval.
 */
const CATALOGUE_MAX_AGE_MS = 6 * 60 * 60 * 1000;

/**
 * The catalogue, for the life of the window.
 *
 * It lives here rather than in the main process because the cost being paid was
 * never the network - it was that the Integrations pane refetched on every
 * mount, and a pane remounts on every tab switch and every navigation. A cache
 * behind IPC still makes the first frame after a remount a spinner, because IPC
 * is a promise. A module-level one is already in hand when the component runs,
 * so a remount draws the list it drew before, with nothing in between.
 *
 * The main process keeps its own cache of tool schemas for a different reader
 * on a different path; `refresh` drops that one too, through the handler.
 */
let catalogue = null;

/**
 * Is a cached catalogue still worth showing.
 *
 * A timestamp in the future is treated as stale rather than fresh-forever: the
 * clock moving backwards should cost one refetch, not lock the window to a
 * catalogue it can never expire.
 */
export function isCatalogueFresh(entry, now = Date.now(), maxAgeMs = CATALOGUE_MAX_AGE_MS) {
  if (!entry || !Array.isArray(entry.items)) return false;
  const age = now - Number(entry.fetchedAt ?? 0);
  return Number.isFinite(age) && age >= 0 && age < maxAgeMs;
}

/**
 * The connections, for the life of the window.
 *
 * They are workspace files and cost a few milliseconds to read, but a few
 * milliseconds is one frame with nothing on it, and what the user sees is the
 * list of their connected apps blinking out and coming back on every visit to
 * this screen. Held here so a remount draws the rows it drew before, and
 * replaced the moment a fresh read lands.
 */
let connectionRows = null;

/** For tests, and for anything that wants the next read to go to the network. */
export function forgetCatalogue() {
  catalogue = null;
  connectionRows = null;
}

export const composio = {
  /**
   * A connection or a catalogue changed, in this window or another.
   *
   * The app has emitted this since the first build and nothing subscribed, so
   * a handshake that went ACTIVE while its dialog was open kept saying it was
   * waiting until the person closed and reopened the screen - which is the
   * exact bug the emitter was written to fix. Returns its own unsubscribe, or
   * a no-op where there is no bridge.
   */
  onEvent(callback) {
    const bridge = api("composioAPI");
    if (!bridge?.onEvent) return () => {};
    return bridge.onEvent(callback);
  },

  /** Whether an API key is stored. False, not an error, when there is none. */
  async configured() {
    const bridge = api("composioAPI");
    if (!bridge) return false;
    return Boolean(unwrap(await bridge.configured()));
  },

  /**
   * The cached catalogue, or null. Synchronous on purpose: it is what lets a
   * remounting screen render the list on its first frame instead of a spinner.
   */
  cachedToolkits() {
    return isCatalogueFresh(catalogue) ? catalogue : null;
  },

  /**
   * The catalogue of apps: `{ items, fetchedAt }`.
   *
   * `refresh: true` goes to Composio whatever is cached. A failed refresh keeps
   * the old list rather than clearing it - the user pressed a button, and
   * losing the catalogue they were reading is a worse answer to a bad network
   * than showing it a few minutes older with the error beside it.
   */
  async toolkits({ refresh = false, ...query } = {}) {
    // Only the whole catalogue is cached. A narrowed read is somebody asking a
    // different question, and answering it out of a cache of everything would
    // hand them rows they filtered out.
    const whole = !query.search && !query.category;
    if (whole && !refresh && isCatalogueFresh(catalogue)) return catalogue;

    const bridge = api("composioAPI");
    if (!bridge) return { items: [], fetchedAt: Date.now(), nextCursor: null };

    const answer = unwrap(await bridge.toolkits({ ...query, refresh }));
    const fetched = {
      items: answer?.items ?? [],
      fetchedAt: Number(answer?.fetchedAt) || Date.now(),
      nextCursor: answer?.nextCursor ?? null,
    };
    if (whole) catalogue = fetched;
    return fetched;
  },

  /** Every operation one toolkit exposes. */
  async tools(toolkitSlug) {
    const bridge = api("composioAPI");
    if (!bridge) return [];
    return unwrap(await bridge.tools(toolkitSlug)) ?? [];
  },

  /** The connections this window last read, or null. Synchronous, like
   *  `cachedToolkits`, and for the same reason: a remount should draw. */
  cachedConnections() {
    return connectionRows;
  },

  async connections() {
    const bridge = api("composioAPI");
    if (!bridge) return [];
    const rows = unwrap(await bridge.connections()) ?? [];
    connectionRows = rows;
    return rows;
  },

  /** Begin an OAuth handshake: `{ id, redirectUrl, status }`. */
  async connect(toolkitSlug) {
    const bridge = api("composioAPI");
    if (!bridge) return unavailable("Connecting an app");
    return unwrap(await bridge.connect(toolkitSlug));
  },

  /** Poll one connection. Returns the stored record with a fresh status. */
  async status(id) {
    const bridge = api("composioAPI");
    if (!bridge) return unavailable("Checking a connection");
    return unwrap(await bridge.status(id));
  },

  async disconnect(id) {
    const bridge = api("composioAPI");
    if (!bridge) return unavailable("Disconnecting an app");
    return unwrap(await bridge.disconnect(id));
  },

  async setPermissions(id, patch) {
    const bridge = api("composioAPI");
    if (!bridge) return unavailable("Saving which tools an app may offer");
    return unwrap(await bridge.setPermissions(id, patch));
  },

  /**
   * Start the handshake again without losing the row.
   *
   * The only remedy before this was disconnect and connect again, which threw
   * away the list of operations the user had chosen - real work, for a big
   * toolkit. Returns `{ id, redirectUrl, status }`, same as `connect`.
   */
  async reconnect(id) {
    const bridge = api("composioAPI");
    if (!bridge) return unavailable("Reconnecting an app");
    return unwrap(await bridge.reconnect(id));
  },

  /**
   * Ask Composio what it actually holds, and write it back to every record.
   *
   * A lapsed OAuth token says nothing when it lapses: no callback arrives, the
   * record still reads ACTIVE, and the first anyone hears of it is a tool
   * failing mid-turn. This is what lets the screen say so first.
   */
  async sync() {
    const bridge = api("composioAPI");
    // Null, not an empty list: a preload without this channel has not told us
    // there are no connections, it has told us nothing, and a caller that
    // cannot tell those apart will render "no apps connected" over a list.
    if (!bridge?.sync) return null;
    const rows = unwrap(await bridge.sync()) ?? [];
    connectionRows = rows;
    return rows;
  },

  /** Drop the cached tool lists, which otherwise live as long as the process. */
  async refresh(toolkitSlug) {
    const bridge = api("composioAPI");
    if (!bridge?.refresh) return null;
    return unwrap(await bridge.refresh(toolkitSlug));
  },
};

/* -- MCP -------------------------------------------------------------------- */

export const mcp = {
  /** Stored servers, already merged with whatever is running. */
  async list() {
    const bridge = api("mcpAPI");
    if (!bridge) return [];
    return unwrap(await bridge.list()) ?? [];
  },

  async add(record) {
    const bridge = api("mcpAPI");
    if (!bridge) return unavailable("Adding an MCP server");
    return unwrap(await bridge.add(record));
  },

  /**
   * Change a stored server.
   *
   * The `mcp:update` handler in the main process takes `(id, changes)`, but the
   * preload bridge forwards a single argument, so a two-argument call arrives
   * as an id with nothing to apply. Rather than silently no-op, the record is
   * patched through the workspace bridge - which writes the same file the
   * handler would have written - and the two things the handler does afterwards
   * are done here: drop the live connection, because whatever is running is
   * running on the old config, and drop the main process's tool cache.
   *
   * The arity check means this repairs itself the day the bridge is fixed.
   */
  async update(id, changes) {
    const bridge = api("mcpAPI");
    if (!bridge) return unavailable("Editing an MCP server");
    if (bridge.update.length >= 2) return unwrap(await bridge.update(id, changes));

    const saved = await workspaceClient().patch(MCP_COLLECTION, id, changes ?? {});
    try {
      unwrap(await bridge.disconnect(id));
    } catch {
      /* a server that was not running is not a failed edit */
    }
    await invalidateTools();
    return saved;
  },

  async remove(id) {
    const bridge = api("mcpAPI");
    if (!bridge) return unavailable("Removing an MCP server");
    return unwrap(await bridge.remove(id));
  },

  async connect(id) {
    const bridge = api("mcpAPI");
    if (!bridge) return unavailable("Connecting an MCP server");
    return unwrap(await bridge.connect(id));
  },

  async disconnect(id) {
    const bridge = api("mcpAPI");
    if (!bridge) return unavailable("Disconnecting an MCP server");
    return unwrap(await bridge.disconnect(id));
  },

  /** Start a server, read its tools, stop it again. Never leaves it running. */
  async test(record) {
    const bridge = api("mcpAPI");
    if (!bridge) return unavailable("Testing an MCP server");
    return unwrap(await bridge.test(record));
  },

  async status() {
    const bridge = api("mcpAPI");
    if (!bridge) return [];
    return unwrap(await bridge.status()) ?? [];
  },

  /** What one connected server provides. Empty when it is not connected. */
  async tools(id) {
    const bridge = api("mcpAPI");
    if (!bridge) return [];
    return unwrap(await bridge.tools(id)) ?? [];
  },
};

/* -- OpenAPI ---------------------------------------------------------------- */

export const openapi = {
  async list() {
    const bridge = api("openapiAPI");
    if (!bridge) return [];
    return unwrap(await bridge.list()) ?? [];
  },

  /** Import a spec by URL or text. Returns `{ record, operations, warnings }`. */
  async import(input) {
    const bridge = api("openapiAPI");
    if (!bridge) return unavailable("Importing a spec");
    return unwrap(await bridge.import(input));
  },

  async remove(id) {
    const bridge = api("openapiAPI");
    if (!bridge) return unavailable("Removing an import");
    return unwrap(await bridge.remove(id));
  },

  /** Same bridge arity problem as `mcp.update`, and the same repair. */
  async update(id, changes) {
    const bridge = api("openapiAPI");
    if (!bridge) return unavailable("Editing an import");
    if (bridge.update.length >= 2) return unwrap(await bridge.update(id, changes));

    const saved = await workspaceClient().patch(OPENAPI_COLLECTION, id, changes ?? {});
    await invalidateTools();
    return saved;
  },

  async operations(id) {
    const bridge = api("openapiAPI");
    if (!bridge) return [];
    return unwrap(await bridge.operations(id)) ?? [];
  },

  async setOperations(id, operations) {
    const bridge = api("openapiAPI");
    if (!bridge) return unavailable("Saving which operations are offered");
    return unwrap(await bridge.setOperations(id, operations));
  },

  /**
   * What the spec says about itself: `{ servers, security, warnings }`.
   *
   * Both halves are decisions the user has to make - which server to call, and
   * whether the authentication this API wants is one that can be set up here at
   * all - and both live only in the stored document.
   */
  async details(id) {
    const bridge = api("openapiAPI");
    if (!bridge?.details) return null;
    return unwrap(await bridge.details(id));
  },

  /** Call one operation for real, with arguments the user typed. */
  async test(id, operationId, args) {
    const bridge = api("openapiAPI");
    if (!bridge) return unavailable("Calling an operation");
    return unwrap(await bridge.test(id, operationId, args ?? {}));
  },
};
