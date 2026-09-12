import { call, subscribe } from "./envelope";

/**
 * The workspace bridge.
 *
 * A literal mirror of the `ws:*` commands in `src-tauri/src/ws.rs` - no
 * caching, no massaging of the envelope. `lib/workspace.jsx` is where policy
 * lives; if this file started making decisions there would be two places to
 * look when a read came back wrong.
 *
 * Tauri names its arguments, so each method maps its positional parameters onto
 * the object the command expects. That mapping is the only thing here, and it
 * is why a renamed Rust parameter shows up as a compile-time-shaped bug in one
 * line rather than as a silently undefined argument at runtime.
 */
export function workspaceBridge() {
  return {
    status: () => call("ws_status"),
    layout: () => call("ws_layout"),
    tree: () => call("ws_tree"),

    browse: (startAt) => call("ws_browse", { startAt: startAt ?? null }),
    chooseFolder: (options) => call("ws_choose_folder", { options: options ?? null }),
    inspect: (dir) => call("ws_inspect", { dir: String(dir ?? "") }),
    configure: (dir) => call("ws_configure", { dir: String(dir ?? "") }),
    reset: () => call("ws_reset"),
    reveal: (relPath) => call("ws_reveal", { relPath: relPath ?? "" }),
    listDir: (relPath) => call("ws_list_dir", { relPath: relPath ?? "" }),

    list: (name) => call("ws_list", { name }),
    get: (name, id) => call("ws_get", { name, id }),
    put: (name, record) => call("ws_put", { name, record }),
    patch: (name, id, changes) => call("ws_patch", { name, id, changes }),
    remove: (name, id) => call("ws_remove", { name, id }),
    rename: (name, id, nextId) => call("ws_rename", { name, id, nextId }),

    readDocument: (key, fallback) => call("ws_doc_get", { key, fallback: fallback ?? null }),
    writeDocument: (key, value) => call("ws_doc_set", { key, value }),

    secrets: {
      list: () => call("ws_secret_list"),
      get: (name) => call("ws_secret_get", { name }),
      set: (name, value, label) =>
        call("ws_secret_set", { name, value: String(value ?? ""), label: label ?? "" }),
      remove: (name) => call("ws_secret_remove", { name }),
    },

    readFile: (relPath) => call("ws_file_read", { relPath }),
    writeFile: (relPath, contents) =>
      call("ws_file_write", { relPath, contents: String(contents ?? "") }),
    readBytes: (relPath) => call("ws_file_read_bytes", { relPath }),
    readImage: (filePath) => call("ws_file_read_image", { filePath: String(filePath ?? "") }),
    writeBytes: (relPath, base64) =>
      call("ws_file_write_bytes", { relPath, base64Data: String(base64 ?? "") }),
    removeFile: (relPath) => call("ws_file_remove", { relPath }),
    wipe: () => call("ws_wipe"),

    // Fired whenever the folder changes under the app - another window, or a
    // write from a different pane - so open views can refetch instead of
    // drifting.
    onChanged: (callback) => subscribe("workspace:changed", callback),
  };
}
