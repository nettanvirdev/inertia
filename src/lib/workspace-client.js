/**
 * The window's view of the workspace, and the reason the UI never has to ask
 * whether it is running inside the desktop app.
 *
 * Two adapters implement one interface. `bridgeClient` forwards to the Tauri
 * bridge and is what ships. `memoryClient` keeps the same data in localStorage
 * and is what runs in a plain browser tab - the Vite dev server, a screenshot,
 * a test. Without it every screen behind first-run setup would be unreachable
 * outside the packaged app, which is exactly the UI you most want to iterate on.
 *
 * Both return plain values and throw on failure. Unwrapping the bridge's
 * `{ ok, error }` envelope happens here, once, so no caller repeats it.
 */

const MEMORY_KEY = "inertia.workspace.memory";

/* -- the bridge ------------------------------------------------------- */

function unwrap(reply) {
  if (!reply || typeof reply !== "object") throw new Error("The workspace did not answer.");
  if (reply.ok) return reply.data;
  throw new Error(reply.error || "The workspace refused that.");
}

function bridgeClient(api) {
  const call =
    (fn) =>
    async (...args) =>
      unwrap(await fn(...args));
  return {
    kind: "bridge",
    status: call(api.status),
    layout: call(api.layout),
    tree: call(api.tree),
    browse: call(api.browse),
    chooseFolder: call(api.chooseFolder),
    inspect: call(api.inspect),
    configure: call(api.configure),
    reset: call(api.reset),
    reveal: call(api.reveal),
    listDir: call(api.listDir),
    list: call(api.list),
    get: call(api.get),
    put: call(api.put),
    patch: call(api.patch),
    remove: call(api.remove),
    rename: call(api.rename),
    readDocument: call(api.readDocument),
    writeDocument: call(api.writeDocument),
    secrets: {
      list: call(api.secrets.list),
      get: call(api.secrets.get),
      set: call(api.secrets.set),
      remove: call(api.secrets.remove),
    },
    readFile: call(api.readFile),
    writeFile: call(api.writeFile),
    readBytes: call(api.readBytes),
    readImage: call(api.readImage),
    writeBytes: call(api.writeBytes),
    removeFile: call(api.removeFile),
    wipe: call(api.wipe),
    onChanged: api.onChanged,
  };
}

/* -- the browser stand-in --------------------------------------------- */

function slugify(value, fallback) {
  const slug = String(value ?? "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
  return slug || fallback;
}

function memoryClient() {
  const listeners = new Set();

  const load = () => {
    try {
      return JSON.parse(localStorage.getItem(MEMORY_KEY) ?? "null") ?? blank();
    } catch {
      return blank();
    }
  };
  const blank = () => ({
    configured: false,
    root: null,
    collections: {},
    documents: {},
    secrets: {},
  });
  const save = (state) => {
    try {
      localStorage.setItem(MEMORY_KEY, JSON.stringify(state));
    } catch {
      /* a preview that cannot persist is still a usable preview */
    }
    for (const fn of listeners) fn({ memory: true });
    return state;
  };

  const rows = (state, name) => (state.collections[name] ??= []);

  return {
    kind: "memory",
    async status() {
      const state = load();
      return {
        configured: state.configured,
        root: state.root,
        suggested: String.raw`C:\Users\you\Documents\Inertia`,
        manifest: state.configured ? { app: "inertia", version: 1, id: "preview" } : null,
        preview: true,
      };
    },
    async layout() {
      return { version: 1, directories: [], collections: [], documents: [] };
    },
    async tree() {
      const state = load();
      return { root: state.root, directories: [] };
    },
    async browse() {
      return null; // no folder picker outside the desktop app - the field is typed into
    },
    async inspect(dir) {
      return {
        path: dir,
        exists: false,
        isEmpty: true,
        hasWorkspace: false,
        action: "create",
        writable: true,
        error: null,
      };
    },
    async configure(dir) {
      const state = load();
      state.configured = true;
      state.root = dir;
      save(state);
      return this.status();
    },
    async reset() {
      save(blank());
      return this.status();
    },
    async reveal() {
      return null;
    },
    /** There is no native dialog in a browser preview, and no folder to point
     *  at if there were. Cancelling is the honest answer. */
    async chooseFolder() {
      return null;
    },
    /** There is no disk behind a browser preview, so the folder is empty. */
    async listDir() {
      return [];
    },
    async list(name) {
      // Deduplicated on the way out. In a real workspace an id is a filename
      // and cannot repeat, so nothing downstream was ever written to cope with
      // one that does - React renders two rows with the same key and drops one.
      // This store is an array, and it holds whatever an older build wrote into
      // it: local ids used to restart at one on every reload, so a folder that
      // has been through a few sessions really does have three records calling
      // themselves `thr-local-1`. The newest wins, which is the same rule a
      // folder full of files would have applied by overwriting.
      const seen = new Map();
      for (const row of rows(load(), name)) if (row?.id) seen.set(row.id, row);
      return [...seen.values()];
    },
    async get(name, id) {
      return rows(load(), name).find((r) => r.id === id) ?? null;
    },
    async put(name, record) {
      const state = load();
      const items = rows(state, name);
      const now = new Date().toISOString();
      const id =
        record.id || slugify(record.name ?? record.title, "item") || `item-${items.length + 1}`;
      const index = items.findIndex((r) => r.id === id);
      const next = { ...record, id, createdAt: items[index]?.createdAt ?? now, updatedAt: now };
      if (index >= 0) items[index] = next;
      else items.push(next);
      save(state);
      return next;
    },
    async patch(name, id, changes) {
      const current = await this.get(name, id);
      if (!current) throw new Error(`No such ${name}: ${id}`);
      return this.put(name, { ...current, ...changes, id });
    },
    async remove(name, id) {
      const state = load();
      state.collections[name] = rows(state, name).filter((r) => r.id !== id);
      save(state);
      return { id, removed: true };
    },
    async rename(name, id, nextId) {
      const current = await this.get(name, id);
      if (!current) throw new Error(`No such ${name}: ${id}`);
      await this.remove(name, id);
      return this.put(name, { ...current, id: nextId });
    },
    async readDocument(key, fallback = {}) {
      return load().documents[key] ?? fallback;
    },
    async writeDocument(key, value) {
      const state = load();
      state.documents[key] = value;
      save(state);
      return value;
    },
    secrets: {
      async list() {
        const state = load();
        return Object.entries(state.secrets).map(([name, entry]) => ({
          name,
          label: entry.label ?? "",
          hint: "•••",
          updatedAt: entry.updatedAt ?? null,
        }));
      },
      async get(name) {
        return load().secrets[name]?.value ?? null;
      },
      async set(name, value, label = "") {
        const state = load();
        state.secrets[name] = { value, label, updatedAt: new Date().toISOString() };
        save(state);
        return { name, label, hint: "•••" };
      },
      async remove(name) {
        const state = load();
        delete state.secrets[name];
        save(state);
        return { name, removed: true };
      },
    },
    async readFile() {
      return null;
    },
    async writeFile(relPath) {
      return relPath;
    },
    // A browser preview has no folder to write bytes into, so an avatar picked
    // there stays a data URL in local storage and nothing pretends otherwise.
    async readBytes() {
      return null;
    },
    // Nor a disk to read a picture off: in a browser preview a message that
    // points at a file on the machine shows its link, which is what the
    // renderer already does when an image cannot be read.
    async readImage() {
      return null;
    },
    async writeBytes(relPath) {
      return relPath;
    },
    async removeFile(relPath) {
      return relPath;
    },
    async wipe() {
      window.localStorage.removeItem(MEMORY_KEY);
      return { wiped: true };
    },
    onChanged(callback) {
      listeners.add(callback);
      return () => listeners.delete(callback);
    },
  };
}

/** One instance for the process. The adapter cannot change at runtime. */
let client;

export function workspaceClient() {
  if (!client) {
    const api = typeof window !== "undefined" ? window.workspaceAPI : null;
    client = api ? bridgeClient(api) : memoryClient();
  }
  return client;
}

/** True when the app is running for real, with a folder on a real disk. */
export function isNativeWorkspace() {
  return workspaceClient().kind === "bridge";
}
