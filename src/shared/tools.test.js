import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { TOOLS, TOOL_KEYS, keyForTool, toolByKey } from "./tools.js";
import { matches } from "./permission.js";

/**
 * The catalogue against what the tools actually ask.
 *
 * A key here that no tool asks under is a switch on the settings screen that
 * does nothing, and a tool filed under the wrong key is a rule that never
 * fires. Both read as working, which is why they are pinned.
 */

const RUST = fs.readFileSync(
  path.join(import.meta.dirname, "../../src-tauri/src/inertia_tools.rs"),
  "utf8"
);

describe("the permission catalogue", () => {
  it("names the same keys the agent's own rule tool accepts, in the same order", () => {
    const block = RUST.match(/pub const TOOL_KEYS: &\[&str\] = &\[([\s\S]*?)\];/);
    expect(block).not.toBeNull();
    const rust = [...block[1].matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
    expect(TOOL_KEYS).toEqual(rust);
  });

  it("files each tool under the key it asks under", () => {
    // `glob` and `grep` ask under `read`; `memory_forget` under `memory`.
    expect(keyForTool("glob")).toBe("read");
    expect(keyForTool("grep")).toBe("read");
    expect(keyForTool("memory_forget")).toBe("memory");
    expect(toolByKey("glob")).toBeNull();
  });

  it("lists only browser tools that exist", () => {
    expect(toolByKey("browser").tools).not.toContain("browser_screenshot");
  });

  it("suggests memory rules that match what the memory tools ask about", () => {
    const suggestions = toolByKey("memory").suggestions;
    const covers = (target) => suggestions.find((s) => matches(s.pattern, target))?.action;
    expect(covers("recall")).toBe("allow");
    expect(covers("remember Likes cats")).toBe("allow");
    expect(covers("forget mem_1")).toBe("ask");
  });

  it("files every tool once", () => {
    const ids = TOOLS.flatMap((tool) => tool.tools);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
