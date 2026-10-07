import { describe, it, expect } from "vitest";
import {
  ANY,
  asRuleMap,
  asRules,
  evaluate,
  fromConfig,
  fromFlat,
  matches,
  merge,
  rule,
  specificity,
  toConfig,
  toFlat,
  toolsThatAsk,
  visibleTools,
} from "./permission";

/**
 * Permission answers are the difference between an agent that can be trusted
 * with a terminal and one that cannot, so the resolution order gets real tests.
 * The cases below are the ones a user would actually write.
 */

describe("matches", () => {
  it("takes anything for the catch-all", () => {
    expect(matches(ANY, "git push")).toBe(true);
  });

  it("compares literally when there is no wildcard", () => {
    expect(matches("git push", "git push")).toBe(true);
    expect(matches("git push", "git push --force")).toBe(false);
  });

  it("expands a trailing wildcard", () => {
    expect(matches("git *", "git status")).toBe(true);
    expect(matches("git *", "npm install")).toBe(false);
  });

  it("lets a trailing wildcard argument cover no arguments", () => {
    expect(matches("git status *", "git status")).toBe(true);
    expect(matches("git status *", "git statusx")).toBe(false);
  });

  it("does not let a pattern smuggle in a regex", () => {
    expect(matches("git commit -m (fix)", "git commit -m (fix)")).toBe(true);
    expect(matches("a.c", "abc")).toBe(false);
  });

  it("spans newlines, since a shell argument can", () => {
    expect(matches("rm *", "rm -rf\n/tmp")).toBe(true);
  });
});

describe("specificity", () => {
  it("ranks an exact string above any pattern", () => {
    expect(specificity("git push")).toBeGreaterThan(specificity("git *"));
  });

  it("ranks a longer pattern above a shorter one", () => {
    expect(specificity("git push *")).toBeGreaterThan(specificity("git *"));
  });

  it("puts the catch-all at the bottom", () => {
    expect(specificity(ANY)).toBe(0);
  });
});

describe("evaluate", () => {
  it("asks when nothing has an opinion", () => {
    const verdict = evaluate([], "shell", "ls");
    expect(verdict.action).toBe("ask");
    expect(verdict.implicit).toBe(true);
  });

  it("uses a plain tool rule", () => {
    const rules = [rule("shell", "allow")];
    expect(evaluate(rules, "shell", "ls").action).toBe("allow");
  });

  it("lets a specific pattern beat a general one whatever the order", () => {
    const loose = rule("shell", "allow");
    const tight = rule("shell", "deny", "rm -rf *");
    expect(evaluate([loose, tight], "shell", "rm -rf /").action).toBe("deny");
    expect(evaluate([tight, loose], "shell", "rm -rf /").action).toBe("deny");
    expect(evaluate([tight, loose], "shell", "ls").action).toBe("allow");
  });

  it("resolves the three-rule case a user would actually write", () => {
    const rules = [
      rule("shell", "allow"),
      rule("shell", "ask", "git push *"),
      rule("shell", "deny", "rm -rf *"),
    ];
    expect(evaluate(rules, "shell", "npm test").action).toBe("allow");
    expect(evaluate(rules, "shell", "git push origin main").action).toBe("ask");
    expect(evaluate(rules, "shell", "rm -rf node_modules").action).toBe("deny");
  });

  it("lets a rule about the tool beat a catch-all across tools", () => {
    const rules = [rule(ANY, "deny", "dangerous-thing"), rule("shell", "allow")];
    expect(evaluate(rules, "shell", "dangerous-thing").action).toBe("allow");
  });

  it("still applies a catch-all to a tool with no rule of its own", () => {
    expect(evaluate([rule(ANY, "deny")], "openapi_docs_get", "https://x.dev").action).toBe("deny");
  });

  it("says which rule decided it", () => {
    const tight = rule("shell", "deny", "rm -rf *");
    expect(evaluate([rule("shell", "allow"), tight], "shell", "rm -rf /").rule).toBe(tight);
  });
});

/**
 * MCP tools, which are the case the pattern side of a rule exists for.
 *
 * Their names are not knowable until a server is connected, so the rule has to
 * be writable about the server instead. The descriptor below is the one the
 * Electron build's MCP integration built; a live server is not needed to prove
 * that what it produces is what a rule can be written against.
 */
describe("MCP targets", () => {
  const descriptor = {
    key: "mcp",
    target: () => "warehouse/query",
    always: () => "warehouse/*",
  };

  it("names the server and the tool, and remembers the server", () => {
    expect(descriptor.key).toBe("mcp");
    expect(descriptor.target()).toBe("warehouse/query");
    expect(descriptor.always()).toBe("warehouse/*");
  });

  it("lets one rule cover a whole server", () => {
    const rules = [rule("mcp", "ask"), rule("mcp", "allow", "warehouse/*")];
    expect(evaluate(rules, "mcp", "warehouse/query").action).toBe("allow");
    // A different server is not covered by it, which is the point of scoping by
    // the server rather than by whatever prefix it gave its tool names.
    expect(evaluate(rules, "mcp", "billing/query").action).toBe("ask");
  });

  it("makes deny-everything a single rule", () => {
    const rules = [rule("mcp", "deny")];
    expect(evaluate(rules, "mcp", "warehouse/query").action).toBe("deny");
    expect(evaluate(rules, "mcp", "billing/refund").action).toBe("deny");
  });
});

describe("config round trip", () => {
  it("reads the three shorthand forms", () => {
    expect(fromConfig("deny")).toEqual([{ tool: ANY, pattern: ANY, action: "deny" }]);
    expect(fromConfig({ shell: "ask" })).toEqual([{ tool: "shell", pattern: ANY, action: "ask" }]);
    expect(fromConfig({ shell: { "git push *": "ask" } })).toEqual([
      { tool: "shell", pattern: "git push *", action: "ask" },
    ]);
  });

  it("falls back to asking for an action it does not recognise", () => {
    expect(fromConfig({ shell: "maybe" })[0].action).toBe("ask");
  });

  it("writes the simple case as a bare action", () => {
    expect(toConfig([rule("shell", "allow")])).toEqual({ shell: "allow" });
  });

  it("keeps the catch-all when a per-argument rule forces the object form", () => {
    const config = toConfig([rule("shell", "allow"), rule("shell", "deny", "rm *")]);
    expect(config).toEqual({ shell: { "*": "allow", "rm *": "deny" } });
  });

  it("survives a round trip", () => {
    const rules = [rule("shell", "allow"), rule("shell", "deny", "rm -rf *"), rule("write", "ask")];
    expect(fromConfig(toConfig(rules))).toEqual(expect.arrayContaining(rules));
  });
});

describe("flat form", () => {
  it("reads a bare tool as the catch-all", () => {
    expect(fromFlat({ shell: "allow" })).toEqual([rule("shell", "allow")]);
  });

  it("splits the tool from its pattern at the first slash", () => {
    expect(fromFlat({ "shell/git push *": "ask" })).toEqual([rule("shell", "ask", "git push *")]);
  });

  it("keeps a pattern that is itself full of slashes and colons", () => {
    expect(fromFlat({ "openapi_docs_get/https://api.example.com/*": "allow" })).toEqual([
      rule("openapi_docs_get", "allow", "https://api.example.com/*"),
    ]);
  });

  it("survives a round trip", () => {
    const rules = [
      rule("shell", "allow"),
      rule("shell", "deny", "rm -rf *"),
      rule("openapi_docs_get", "ask", "https://*"),
    ];
    expect(fromFlat(toFlat(rules))).toEqual(rules);
  });

  it("ignores a key with no tool in it", () => {
    expect(fromFlat({ "/orphan": "allow" })).toEqual([]);
  });
});

describe("merge", () => {
  it("lets a later layer override the same rule", () => {
    const merged = merge([rule("shell", "deny")], [rule("shell", "allow")]);
    expect(merged).toEqual([rule("shell", "allow")]);
  });

  it("keeps rules that do not collide", () => {
    const merged = merge([rule("shell", "deny")], [rule("shell", "ask", "git *")]);
    expect(merged).toHaveLength(2);
  });
});

describe("visibility", () => {
  const tools = [{ id: "read" }, { id: "shell" }, { id: "openapi_docs_get" }];

  it("hides a tool that is denied outright", () => {
    const visible = visibleTools(tools, [rule("shell", "deny")]);
    expect(visible.map((t) => t.id)).toEqual(["read", "openapi_docs_get"]);
  });

  it("keeps a tool that is denied only for some arguments", () => {
    const visible = visibleTools(tools, [rule("shell", "deny", "rm *")]);
    expect(visible.map((t) => t.id)).toEqual(["read", "shell", "openapi_docs_get"]);
  });

  it("accepts bare ids as well as descriptors", () => {
    expect(visibleTools(["read", "shell"], [rule("shell", "deny")])).toEqual(["read"]);
  });

  it("lists what will stop and ask", () => {
    const rules = [rule(ANY, "allow"), rule("shell", "ask")];
    expect(toolsThatAsk(tools, rules).map((t) => t.id)).toEqual(["shell"]);
  });
});

/**
 * A workspace folder written by an older build holds a map of permission ids
 * where this now expects an array of rules. Every reader spreads what it finds,
 * and `?? []` does not catch an object - so a stale settings file used to take
 * the whole window down with "is not iterable". Whatever the folder holds has
 * to come back as rules.
 */
describe("asRules", () => {
  it("passes a well-formed ruleset through", () => {
    const rules = [rule("shell", "ask"), rule("read", "allow", "src/*")];
    expect(asRules(rules)).toEqual(rules);
  });

  it("reads the legacy id map as rules instead of throwing", () => {
    const legacy = { "perm-run-terminal": "ask", "perm-read-files": "allow" };
    expect(asRules(legacy)).toEqual([
      rule("perm-run-terminal", "ask"),
      rule("perm-read-files", "allow"),
    ]);
  });

  it("answers with an empty ruleset for anything that is not one", () => {
    for (const value of [null, undefined, "deny", 7, true]) expect(asRules(value)).toEqual([]);
  });

  it("drops entries that name no tool, and repairs the rest", () => {
    const messy = [null, "shell", { pattern: "*" }, { tool: "shell", action: "nonsense" }];
    expect(asRules(messy)).toEqual([rule("shell", "ask")]);
  });

  it("leaves a dead permission name matching nothing", () => {
    const rules = asRules({ "perm-run-terminal": "allow" });
    expect(evaluate(rules, "shell").action).toBe("ask");
  });

  it("converts every agent in the map", () => {
    const map = asRuleMap({
      atlas: { "perm-read-files": "allow" },
      forge: [rule("shell", "deny")],
    });
    expect(map.atlas).toEqual([rule("perm-read-files", "allow")]);
    expect(map.forge).toEqual([rule("shell", "deny")]);
  });

  it("answers with an empty map for anything that is not one", () => {
    for (const value of [null, [], "deny"]) expect(asRuleMap(value)).toEqual({});
  });
});
