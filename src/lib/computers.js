/**
 * The window's view of a machine.
 *
 * A thin, literal mirror of the channels in `main/sandbox/ipc.cjs`, with the
 * envelope unwrapped and one convenience on top: `runCommand`, which pairs the
 * request with the output stream so a terminal does not have to subscribe,
 * filter and unsubscribe by hand every time someone presses Enter.
 *
 * No caching and no policy. The main process is the only thing that knows what
 * a machine is doing, and a second opinion held in the renderer is a second
 * opinion that goes stale the moment someone stops a container from a terminal.
 */

const bridge = typeof window !== "undefined" ? window.computerAPI : null;

export function areComputersAvailable() {
  return Boolean(bridge?.list);
}

function unwrap(result) {
  if (!result) throw new Error("The computers bridge did not answer.");
  if (result.ok === false) throw new Error(result.error ?? "That did not work.");
  // `"data" in result`, not `result.data ?? result`.
  //
  // The envelope is always `{ok, data}`, and `data` is legitimately null for
  // every call that can honestly answer "there is nothing": a screenshot of a
  // machine with no display, a file that is not there. `??` treats that null as
  // "no value" and falls through to returning the ENVELOPE - so the caller gets
  // the object `{ok: true, data: null}`, which is truthy.
  //
  // The pane then set that object as an <img src> and drew a broken image where
  // its own "this machine has no display" message should have been. It looked
  // like a screenshot bug on Daytona and local, and Docker was only spared
  // because it returns a real picture.
  return "data" in result ? result.data : result;
}

const call = (name, ...args) => {
  if (!bridge?.[name]) {
    return Promise.reject(new Error("Computers are only available in the desktop app."));
  }
  return bridge[name](...args).then(unwrap);
};

export const computers = {
  providers: () => call("providers"),
  settings: () => call("settings"),
  saveSettings: (patch) => call("saveSettings", patch),

  image: () => call("image"),
  catalogue: (providerId) => call("catalogue", providerId),
  buildImage: () => call("buildImage"),
  list: () => call("list"),
  refresh: (id) => call("refresh", id),
  create: (draft) => call("create", draft),
  start: (id) => call("start", id),
  stop: (id) => call("stop", id),
  pause: (id) => call("pause", id),
  resume: (id) => call("resume", id),
  remove: (id) => call("remove", id),

  exec: (id, request) => call("exec", id, request),
  cancel: (runId) => call("cancel", runId),
  listDir: (id, path) => call("listDir", id, path),
  readFile: (id, path) => call("readFile", id, path),
  /** A file's raw bytes as a base64 string - for images and downloads. */
  readFileBytes: (id, path) => call("readFileBytes", id, path),
  /** Save a file from the machine onto the user's own disk, via a save dialog. */
  downloadFile: (id, path) => call("downloadFile", id, path),
  writeFile: (id, path, content) => call("writeFile", id, path, content),

  snapshot: (id, name) => call("snapshot", id, name),
  snapshots: (id) => call("snapshots", id),
  restore: (id, snapshotId) => call("restore", id, snapshotId),
  /** Where the machine's screen can be watched and driven, or null. */
  screen: (id) => call("screen", id),
  screenshot: (id) => call("screenshot", id),
  /** open, close, pageText, windows, click, type, key, scroll. */
  drive: (id, action, args) => call("drive", id, action, args),

  assign: (id, agentIds) => call("assign", id, agentIds),

  /** Browser profiles on this desktop, with how many cookies each holds. */
  cookieSources: () => call("cookieSources"),
  /** Copy one profile's cookies into the machine's browser. */
  importCookies: (id, options) => call("importCookies", id, options),

  onEvent: (handler) => bridge?.onEvent?.(handler) ?? (() => {}),
};

/**
 * Run a command and watch its output.
 *
 * The subscription is opened before the request is sent, because a fast command
 * finishes before the invoke resolves and its output would otherwise arrive
 * with nobody listening - the same race the agent bridge has, with the same
 * answer.
 *
 * Output is matched by command rather than by run id for the first events,
 * since the id only comes back with the answer. In practice the `start` event
 * carries it and arrives first; the filter below is what makes that reliable
 * rather than merely usual.
 */
export function runCommand(computerId, request, { onOutput } = {}) {
  let runId = null;
  const held = [];

  const off = computers.onEvent((event) => {
    if (event.computerId !== computerId) return;
    if (event.type === "start" && !runId) {
      runId = event.runId;
      for (const chunk of held.splice(0)) onOutput?.(chunk);
      return;
    }
    if (event.type !== "output") return;
    if (!runId) held.push(event);
    else if (event.runId === runId) onOutput?.(event);
  });

  const promise = computers
    .exec(computerId, request)
    .finally(() => {
      off?.();
    });

  return {
    promise,
    cancel: () => (runId ? computers.cancel(runId) : Promise.resolve({ cancelled: false })),
  };
}

/**
 * Where a pane should open on this machine.
 *
 * Recorded when the machine was made, because it is a different folder on every
 * provider - /workspace in our container, the sandbox user's home on Daytona.
 * The fallbacks are for records written before it was stored.
 */
export function workdirOf(computer) {
  if (computer?.workdir) return computer.workdir;
  if (computer?.provider === "daytona") return "/home/daytona";
  return "/workspace";
}

/** `/workspace/src/lib` -> the crumbs a breadcrumb bar renders. */
export function pathCrumbs(path) {
  const parts = String(path ?? "/")
    .split("/")
    .filter(Boolean);
  const crumbs = [{ name: "/", path: "/" }];
  let acc = "";
  for (const part of parts) {
    acc += `/${part}`;
    crumbs.push({ name: part, path: acc });
  }
  return crumbs;
}

export function joinPath(base, name) {
  const left = String(base ?? "/").replace(/\/+$/, "");
  return `${left}/${name}`;
}

export function parentPath(path) {
  const parts = String(path ?? "/").split("/").filter(Boolean);
  parts.pop();
  return `/${parts.join("/")}`;
}
