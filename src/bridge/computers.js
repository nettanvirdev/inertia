import { call, subscribe } from "./envelope";

/**
 * The computers bridge: machines an agent can drive.
 *
 * Three providers sit behind this - a container on this laptop, a sandbox in
 * Daytona's cloud, and this machine itself - and nothing here names which. The
 * record says who owns a machine and the backend picks accordingly, so a screen
 * written against these methods works the same whichever answered.
 *
 * The one method with no backend is `importCookies`: lifting a signed-in
 * session out of a browser profile on this desktop means reading a SQLite
 * cookie store and decrypting it against the platform keychain - DPAPI on
 * Windows, the login keyring elsewhere - and none of the crates that do either
 * are in this workspace yet. It answers a refusal rather than being absent,
 * because `lib/computers.js` binds this namespace once and calls through it
 * without checking each method.
 */
const NOT_YET = (what) =>
  `${what} is not wired up in this build yet. The machine itself works - running commands, files, snapshots, the screen - but this part has no backend.`;

export function computersBridge() {
  return {
    providers: () => call("computer_providers"),
    settings: () => call("computer_settings"),
    saveSettings: (patch) => call("computer_save_settings", { patch: patch ?? {} }),

    list: () => call("computer_list"),
    refresh: (id) => call("computer_refresh", { id }),
    create: (draft) => call("computer_create", { draft: draft ?? {} }),

    start: (id) => call("computer_start", { id }),
    stop: (id) => call("computer_stop", { id }),
    pause: (id) => call("computer_pause", { id }),
    resume: (id) => call("computer_resume", { id }),
    remove: (id) => call("computer_remove", { id }),

    exec: (id, request) => call("computer_exec", { id, request: request ?? {} }),
    listDir: (id, path) => call("computer_list_dir", { id, path: path ?? "" }),
    readFile: (id, path) => call("computer_read_file", { id, path }),
    readFileBytes: (id, path) => call("computer_read_file_bytes", { id, path }),
    downloadFile: (id, path) => call("computer_download_file", { id, path }),
    writeFile: (id, path, content) =>
      call("computer_write_file", { id, path, content: String(content ?? "") }),

    snapshot: (id, name) => call("computer_snapshot", { id, name: name ?? null }),
    snapshots: (id) => call("computer_snapshots", { id }),
    restore: (id, snapshotId) => call("computer_restore", { id, snapshotId }),
    screenshot: (id) => call("computer_screenshot", { id }),

    assign: (id, agentIds) => call("computer_assign", { id, agentIds: agentIds ?? [] }),

    /*
     * The sandbox image. The Dockerfile and everything it copies are compiled
     * into the binary, so `image()` can always answer "buildable" and the build
     * needs nothing on disk to find - see `crates/inertia-computers/src/image.rs`
     * for why that beat shipping the folder as a Tauri resource.
     *
     * `buildImage` resolves when the build does and streams `computer:event`
     * while it runs, which is what lets the settings pane paint a log during
     * the fifteen minutes a cold build takes.
     */
    image: () => call("computer_image"),
    buildImage: () => call("computer_build_image"),
    catalogue: (providerId) => call("computer_catalogue", { providerId: providerId ?? null }),

    /** A command already finished by the time `exec` answers, so nothing cancels. */
    cancel: async () => ({ ok: true, data: { cancelled: false } }),

    /*
     * The live screen, and driving it.
     *
     * `screen` answers the two noVNC addresses the machine published, or null
     * for one that has no display - the pane falls back to still frames on
     * null, so this must not throw for an ordinary machine. The webview opens
     * them directly: they are on loopback with no header to add, so there is no
     * proxy in the way.
     */
    screen: (id) => call("computer_screen", { id }),
    drive: (id, action, args) => call("computer_drive", { id, action, args: args ?? {} }),

    /*
     * Reading cookies out of a browser profile on this computer.
     *
     * Empty rather than a refusal: the dialog draws "no browser profiles" from
     * an empty list, which is the truth about what this build can see.
     */
    cookieSources: async () => ({ ok: true, data: [] }),
    importCookies: async () => ({ ok: false, error: NOT_YET("Importing cookies") }),

    onEvent: (callback) => subscribe("computer:event", callback),
  };
}
