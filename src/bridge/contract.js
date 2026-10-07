/**
 * What the renderer is entitled to find on `window`.
 *
 * Transcribed from the preload of the Electron app this one is a port of, and
 * now the authoritative contract itself: the renderer was written against
 * exactly these namespaces and exactly these method names, and calls most of
 * them without checking first.
 *
 * It is data rather than prose so a test can hold the bridge to it. Every
 * namespace-shaped bug so far has been of one of two kinds - a namespace that
 * was never installed, or a method quietly missing from one that was - and both
 * showed up as a blank pane or a raw `is not a function` several screens away
 * from the cause. A list that can be checked turns both into a failing test.
 *
 * `status` records what is actually behind each namespace today:
 *   "live"    - fully implemented against a Rust backend
 *   "partial" - installed, with named methods that answer "not yet"
 *   "absent"  - deliberately not installed, because there is no backend at all
 *
 * "absent" is a real answer, not a TODO marker: the renderer has a guarded path
 * for a missing namespace and draws an honest "needs the desktop app" panel. A
 * stub answering `[]` would instead look like a working feature with nothing in
 * it. What is NOT acceptable is a namespace listed "live" or "partial" that is
 * missing a method the renderer calls, which is what the test enforces.
 */

export const CONTRACT = {
  electronAPI: {
    status: "live",
    methods: [
      "minimize",
      "maximize",
      "close",
      "beginDrag",
      "onWindowState",
      "getAppInfo",
      "openExternal",
      "showInFolder",
      "fetchImage",
      "runSnippet",
      "canRun",
      "readClipboard",
      "saveImage",
      "setZoom",
    ],
  },

  workspaceAPI: {
    status: "live",
    methods: [
      "status",
      "layout",
      "tree",
      "browse",
      "chooseFolder",
      "inspect",
      "configure",
      "reset",
      "reveal",
      "listDir",
      "list",
      "get",
      "put",
      "patch",
      "remove",
      "rename",
      "readDocument",
      "writeDocument",
      "readFile",
      "writeFile",
      "readBytes",
      "readImage",
      "writeBytes",
      "removeFile",
      "wipe",
      "onChanged",
    ],
    // `secrets` is a nested object rather than a flat method.
    nested: { secrets: ["list", "get", "set", "remove"] },
  },

  llmAPI: {
    status: "live",
    methods: ["providers", "models", "test", "chat", "cancel", "cancelAll", "onEvent"],
  },

  agentAPI: {
    status: "partial",
    methods: [
      "run",
      "cancel",
      "cancelAll",
      "steer",
      "forget",
      "active",
      "record",
      "history",
      "summarize",
      "rules",
      "tools",
      "invalidate",
      "openPath",
      "reply",
      "waiting",
      "grants",
      "revoke",
      "questions",
      "answer",
      "dismiss",
      "onEvent",
    ],
    unimplemented: [],
  },

  mcpAPI: {
    status: "live",
    methods: ["list", "add", "update", "remove", "connect", "disconnect", "test", "status", "tools"],
  },

  openapiAPI: {
    status: "live",
    methods: [
      "list",
      "import",
      "remove",
      "update",
      "operations",
      "setOperations",
      "test",
      "details",
    ],
  },

  composioAPI: {
    status: "live",
    methods: [
      "configured",
      "toolkits",
      "tools",
      "logo",
      "connections",
      "connect",
      "status",
      "disconnect",
      "permissions",
      "setPermissions",
      "reconnect",
      "sync",
      "refresh",
      "onEvent",
    ],
  },

  projectAPI: {
    status: "live",
    methods: ["status", "create", "decline", "decide"],
  },

  /*
   * The rest have no backend in this shell yet. Each names what it would take,
   * so the next person does not have to work it out from the method list.
   */

  voiceAPI: {
    status: "live",
    methods: ["configured", "test", "voices", "models", "speak", "transcribe"],
  },

  // A real pty per tab, through `portable-pty`. The pane's two shapes are one
  // here: `capability()` always answers yes, because there is no native module
  // to fail to load and so no piped fallback to draw instead.
  terminalAPI: {
    status: "live",
    methods: ["capability", "open", "write", "resize", "run", "interrupt", "read", "setCwd", "close", "list", "onEvent"],
  },

  // The browser pane: a native child webview layered over this window and
  // positioned from the rectangle the pane measures for it. Two gaps, both
  // answering a sentence rather than being missing - a Tauri webview has no
  // capture API to photograph a page with, and reading a browser profile's
  // cookies off this desktop needs a SQLite reader and the platform keychain,
  // which is the same thing the computers bridge is still missing.
  previewAPI: {
    status: "partial",
    methods: ["open", "navigate", "history", "place", "state", "list", "screenshot", "consoleLog", "networkLog", "devTools", "close", "cookieSources", "importCookies", "onEvent"],
    unimplemented: ["screenshot"],
  },

  computerAPI: {
    status: "partial",
    methods: ["providers", "settings", "saveSettings", "image", "catalogue", "buildImage", "list", "refresh", "create", "start", "stop", "pause", "resume", "remove", "exec", "cancel", "listDir", "readFile", "readFileBytes", "downloadFile", "writeFile", "snapshot", "snapshots", "restore", "screen", "screenshot", "drive", "assign", "cookieSources", "importCookies", "onEvent"],
    // Only the cookie import is left: it needs a SQLite reader and the platform
    // keychain to decrypt what it reads, neither of which this build has. The
    // machine, the image build, the screen and driving it are all live.
    unimplemented: [],
  },

  // Sub-agents. `spawn` starts a run and the table behind it is live, so the
  // panel reads real runs; pausing and restarting one have no machinery yet
  // and say so rather than pretending to have happened.
  crewAPI: {
    status: "partial",
    methods: ["snapshot", "cancel", "pause", "resume", "restart", "interrupt", "followUp", "timeline", "watch", "forget", "onEvent"],
    unimplemented: ["pause", "resume", "restart"],
  },

  // Lifecycle hooks. `PreToolUse`, `PostToolUse` and `Stop` fire in the turn;
  // a `prompt` handler still answers "no model available", because putting the
  // turn's own provider back into a hook is a re-entrancy question worth
  // answering on purpose rather than in passing.
  hooksAPI: { status: "partial", methods: ["list", "recent", "writeExample", "setEnabled", "reveal", "onRun"] },

  // What an agent knows before the conversation starts. The Memory screen
  // itself goes through the `memory` collection on the workspace bridge, which
  // the backend routes across the workspace and the project's own
  // `.inertia/memory/`; this namespace is the rest - search, which project is
  // open, and running a capture pass on demand.
  memoryAPI: {
    status: "live",
    methods: ["status", "setProject", "recall", "save", "forget", "captureNow"],
  },

  // Needs a snapshot of the working folder before a turn's first write.
  // A content-addressed snapshot of the working folder, taken the first time a
  // turn is about to write something. What fills the strip under a reply, and
  // what Undo puts back.
  snapshotAPI: { status: "live", methods: ["available", "diff", "revert"] },

  // A scheduler that keeps time whether a window exists or not: a tick loop in
  // the app process reads the routines folder, writes each enabled routine its
  // next run, and starts a turn for anything due - in the routine's own
  // conversation, as the routine's own agent.
  routineAPI: { status: "live", methods: ["run", "next", "describe", "tick", "onEvent"] },

  // A notice about work that finished, failed, or is blocked waiting for a
  // yes, while the person was looking at something else. The window's focus
  // decides whether it is also an operating-system banner.
  notifyAPI: { status: "live", methods: ["onEvent", "choices", "setChoices"] },

  // What Inertia does with no window: close to the tray rather than quit, and
  // start with the machine. Neither answer lives in the preferences store -
  // one is in `settings.app` and the other is a registry entry the OS owns.
  backgroundAPI: {
    status: "live",
    methods: ["get", "setMinimiseToTray", "setLaunchAtLogin"],
  },
};
