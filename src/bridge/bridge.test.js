import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { CONTRACT } from "./contract";
import { agentBridge } from "./agent";
import { appBridge } from "./app";
import { backgroundBridge } from "./background";
import { notifyBridge } from "./notify";
import { llmBridge } from "./llm";
import { workspaceBridge } from "./workspace";
import { composioBridge, mcpBridge, openapiBridge } from "./integrations";
import { previewBridge } from "./preview";
import { projectBridge } from "./project";
import { voiceBridge } from "./voice";
import { computersBridge } from "./computers";
import { crewBridge } from "./crew";
import { hooksBridge } from "./hooks";
import { memoryBridge } from "./memory";
import { routinesBridge } from "./routines";
import { snapshotBridge } from "./snapshot";
import { terminalBridge } from "./terminal";

/**
 * The bridge against the contract the renderer was written to.
 *
 * This exists because every bridge bug so far has been found by the user, in a
 * screenshot, several screens away from its cause - a pane reading
 * "needs the desktop app" because a namespace was never installed, or a raw
 * `api.chat is not a function` because one method was missed inside a namespace
 * that was. Both are trivially checkable and neither was being checked.
 *
 * The renderer calls most of these without guarding, so "present and callable"
 * is the property that matters. What they return is the business of the tests
 * around the Rust commands behind them.
 */

const BRIDGES = {
  electronAPI: appBridge,
  workspaceAPI: workspaceBridge,
  llmAPI: llmBridge,
  agentAPI: agentBridge,
  mcpAPI: mcpBridge,
  openapiAPI: openapiBridge,
  composioAPI: composioBridge,
  previewAPI: previewBridge,
  projectAPI: projectBridge,
  voiceAPI: voiceBridge,
  computerAPI: computersBridge,
  crewAPI: crewBridge,
  hooksAPI: hooksBridge,
  memoryAPI: memoryBridge,
  routineAPI: routinesBridge,
  snapshotAPI: snapshotBridge,
  terminalAPI: terminalBridge,
  notifyAPI: notifyBridge,
  backgroundAPI: backgroundBridge,
};

beforeEach(() => {
  // `appBridge` attaches a document listener for drag regions, and several
  // bridges read `window` when they are built.
  globalThis.window ??= globalThis;
  globalThis.document ??= { addEventListener() {}, getElementById: () => null };
});

afterEach(() => {
  delete globalThis.__TAURI_INTERNALS__;
});

describe("every namespace the renderer can reach", () => {
  const installed = Object.entries(CONTRACT).filter(([, spec]) => spec.status !== "absent");

  it.each(installed)("%s is built by this shell", (name) => {
    expect(BRIDGES[name], `${name} is contracted as installed but has no builder`).toBeTypeOf(
      "function"
    );
  });

  it.each(installed)("%s implements every method it promises", (name, spec) => {
    const bridge = BRIDGES[name]();
    const missing = spec.methods.filter((method) => typeof bridge[method] !== "function");
    expect(missing, `${name} is missing: ${missing.join(", ")}`).toEqual([]);
  });

  it.each(installed.filter(([, spec]) => spec.nested))(
    "%s implements its nested groups",
    (name, spec) => {
      const bridge = BRIDGES[name]();
      for (const [group, methods] of Object.entries(spec.nested)) {
        expect(bridge[group], `${name}.${group} is missing`).toBeTypeOf("object");
        const missing = methods.filter((m) => typeof bridge[group][m] !== "function");
        expect(missing, `${name}.${group} is missing: ${missing.join(", ")}`).toEqual([]);
      }
    }
  );
});

describe("the honest-gap rule", () => {
  /**
   * A method listed as unimplemented must still answer the envelope its caller
   * unwraps. `lib/llm.js` and `lib/agent.js` both pick their adapter on whether
   * the namespace exists at all and then call straight through, so a gap has to
   * be a readable sentence rather than a TypeError.
   */
  it.each(
    Object.entries(CONTRACT)
      .filter(([, spec]) => spec.unimplemented?.length)
      .flatMap(([name, spec]) => spec.unimplemented.map((method) => [name, method]))
  )("%s.%s refuses with a sentence rather than throwing", async (name, method) => {
    const reply = await BRIDGES[name]()[method]("x", "y", "z");
    expect(reply, `${name}.${method} answered nothing`).toBeTypeOf("object");
    if (reply.ok === false) {
      expect(reply.error, `${name}.${method} refused without saying why`).toBeTruthy();
    }
  });
});

describe("namespaces with no backend", () => {
  /**
   * These are absent on purpose. The renderer draws a panel explaining the
   * feature is unavailable; a stub answering an empty list would instead look
   * like a working screen with nothing in it, and the bug would be reported
   * months later as "it stopped working".
   */
  it("are not quietly installed", () => {
    const absent = Object.entries(CONTRACT)
      .filter(([, spec]) => spec.status === "absent")
      .map(([name]) => name);

    for (const name of absent) {
      expect(BRIDGES[name], `${name} is contracted absent but has a builder`).toBeUndefined();
    }
  });

  /** The list is a statement about this build, so it has to stay accurate. */
  it("are the ones still to be written", () => {
    const absent = Object.entries(CONTRACT)
      .filter(([, spec]) => spec.status === "absent")
      .map(([name]) => name)
      .sort();

    expect(absent).toEqual([]);
  });
});
