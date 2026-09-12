import { hasTauri } from "./envelope";
import { agentBridge } from "./agent";
import { appBridge } from "./app";
import { backgroundBridge } from "./background";
import { computersBridge } from "./computers";
import { crewBridge } from "./crew";
import { hooksBridge } from "./hooks";
import { memoryBridge } from "./memory";
import { notifyBridge } from "./notify";
import { composioBridge, mcpBridge, openapiBridge } from "./integrations";
import { llmBridge } from "./llm";
import { previewBridge } from "./preview";
import { projectBridge } from "./project";
import { routinesBridge } from "./routines";
import { snapshotBridge } from "./snapshot";
import { terminalBridge } from "./terminal";
import { voiceBridge } from "./voice";
import { workspaceBridge } from "./workspace";

/**
 * Installs the bridges the renderer expects on `window`.
 *
 * The renderer was written against Electron's preload contract: a handful of
 * `window.<something>API` namespaces, each a thin forwarder to the main
 * process. This module puts the same namespaces there, backed by Tauri
 * commands, so that not one line of the UI has to know which shell it is
 * running inside.
 *
 * # Why absent rather than stubbed
 *
 * Only the namespaces with a real backend are installed. The rest are left
 * undefined on purpose, and the renderer is already written for that: every
 * consumer reads its bridge through a guarded accessor
 * (`window.voiceAPI ?? null`) and has a path for "not available here". An empty
 * Voice pane that says so is the truth; a stub that answers `[]` is a screen
 * that looks like a working feature with nothing behind it, and the bug it
 * produces is reported months later as "dictation stopped working".
 *
 * As each backend lands, its namespace is added here and the feature lights up
 * with no change to the renderer.
 *
 * # Why this must run first
 *
 * Two modules - `lib/computers.js` and `lib/routines.js` - read their bridge at
 * module-evaluation time rather than per call. So this is imported before
 * anything else in `main.jsx`; ES modules evaluate in import order, and an
 * import moved below `App` would leave those two holding `undefined` forever.
 */
export function installBridges() {
  if (typeof window === "undefined") return;

  // In a plain browser tab there is no backend to forward to. Installing
  // namespaces that could only fail would be worse than installing none: the
  // renderer's own fallbacks - `memoryClient` for the workspace, the mock
  // `electronAPI` in `preview.jsx` - are what make the dev server usable.
  if (!hasTauri()) return;

  window.electronAPI ??= appBridge();
  window.workspaceAPI ??= workspaceBridge();
  window.agentAPI ??= agentBridge();
  window.llmAPI ??= llmBridge();
  window.projectAPI ??= projectBridge();
  window.voiceAPI ??= voiceBridge();
  window.computerAPI ??= computersBridge();
  window.hooksAPI ??= hooksBridge();
  window.memoryAPI ??= memoryBridge();
  window.routineAPI ??= routinesBridge();
  window.snapshotAPI ??= snapshotBridge();
  window.terminalAPI ??= terminalBridge();
  // Being told about work that finished while the window was somewhere else,
  // and the two switches that decide whether there is an app to tell you at
  // all once the window is closed.
  window.notifyAPI ??= notifyBridge();
  window.backgroundAPI ??= backgroundBridge();
  // The agents an agent started. Live because the run table is; three of its
  // buttons answer "not yet" rather than being missing, so the panel is a
  // panel rather than a "needs the desktop app" placeholder.
  window.crewAPI ??= crewBridge();

  // The browser beside the conversation. A native child webview parked over a
  // hole the pane measures, so this namespace existing is the difference
  // between a Browser panel and a "needs the desktop app" placeholder.
  window.previewAPI ??= previewBridge();

  // All three together. `isDesktop()` in `lib/integrations.js` is the AND of
  // them, so installing two would leave the Integrations screen exactly as
  // blank as installing none.
  window.mcpAPI ??= mcpBridge();
  window.openapiAPI ??= openapiBridge();
  window.composioAPI ??= composioBridge();
}

installBridges();
