import { describe, expect, it } from "vitest";

import {
  INJECT_BUDGET_BYTES,
  applicable,
  forPrompt,
  isStale,
  isStorable,
  findDuplicate,
  looksSecret,
  merged,
  sameFolder,
  similarity,
  search,
  summarise,
} from "./memory.js";

const memory = (title, body, extra = {}) => ({
  id: title.toLowerCase().replace(/\W+/g, "-"),
  title,
  body,
  tags: [],
  ...extra,
});

describe("finding the memories a question is about", () => {
  const store = [
    memory(
      "Deploys from master",
      "Inertia work is pushed straight to master, never a feature branch."
    ),
    memory("Package manager", "The project uses pnpm. npm lockfiles are deleted on sight.", {
      tags: ["tooling"],
    }),
    memory("Coffee", "The user takes their coffee black."),
  ];

  it("finds a memory by the words in it", () => {
    expect(search(store, "which branch do we push to").map((m) => m.id)).toContain(
      "deploys-from-master"
    );
  });

  it("finds one by its tags", () => {
    expect(search(store, "tooling").map((m) => m.id)).toContain("package-manager");
  });

  it("returns nothing when it knows nothing, rather than the least unrelated thing", () => {
    // The failure that makes a memory system start lying: a query about
    // something never discussed must not come back with the closest three
    // memories and a confident score.
    expect(search(store, "kubernetes ingress certificates")).toEqual([]);
  });

  it("has nothing to say about an empty question", () => {
    expect(search(store, "")).toEqual([]);
    expect(search(store, "   ")).toEqual([]);
  });

  it("weighs a word in the title above the same word in a body", () => {
    const store2 = [
      memory("Testing", "Nothing much to say here."),
      memory("Unrelated", "A long body that mentions testing exactly once in passing."),
    ];
    expect(search(store2, "testing")[0].id).toBe("testing");
  });
});

describe("what goes into the prompt", () => {
  it("puts pinned memories first", () => {
    const chosen = forPrompt([memory("Ordinary", "a"), memory("Pinned", "b", { pinned: true })]);
    expect(chosen[0].title).toBe("Pinned");
  });

  it("stops at the byte budget rather than at a count", () => {
    const many = Array.from({ length: 200 }, (_, i) =>
      memory(`Memory number ${i}`, "x".repeat(400))
    );
    const chosen = forPrompt(many);
    const spent = chosen.reduce((sum, m) => sum + `${m.title}: ${summarise(m)}`.length, 0);
    expect(spent).toBeLessThanOrEqual(INJECT_BUDGET_BYTES + 400);
    expect(chosen.length).toBeLessThan(many.length);
  });

  it("never drops a pinned memory for being long", () => {
    // The user pinned it. The budget decides what fits around it, not whether
    // the thing they explicitly asked for survives.
    const many = [
      ...Array.from({ length: 50 }, (_, i) => memory(`Filler ${i}`, "y".repeat(300))),
      memory("Pinned and enormous", "z".repeat(4000), { pinned: true }),
    ];
    expect(forPrompt(many).some((m) => m.title === "Pinned and enormous")).toBe(true);
  });

  it("collapses a body into one line, however it was written", () => {
    expect(summarise(memory("t", "first\n\nsecond   line"))).toBe("first second line");
  });

  it("prefers a written description to the opening of the body", () => {
    expect(summarise({ description: "The short version.", body: "The long one." })).toBe(
      "The short version."
    );
  });
});

describe("keeping one project out of another", () => {
  const here = "D:/work/api";
  const there = "D:/work/website";

  const store = [
    memory("Global habit", "Always run the tests before pushing.", { scope: "global" }),
    memory("API convention", "Handlers live in src/routes.", { scope: "project", folder: here }),
    memory("Site convention", "Styles are Tailwind only.", { scope: "project", folder: there }),
  ];

  it("gives a project its own memories and the global ones", () => {
    expect(applicable(store, here).map((m) => m.title)).toEqual(["Global habit", "API convention"]);
  });

  it("never leaks one project's memories into another", () => {
    expect(applicable(store, there).map((m) => m.title)).not.toContain("API convention");
  });

  it("gives a folder with no memories only the global ones", () => {
    expect(applicable(store, "D:/somewhere/else").map((m) => m.title)).toEqual(["Global habit"]);
  });

  it("matches the same folder written differently", () => {
    // The same folder legitimately arrives with either slash and either case of
    // drive letter, and a memory invisible in its own project because of a
    // slash would be unexplainable to anyone looking at it.
    expect(sameFolder("D:\\work\\api", "d:/work/api")).toBe(true);
    expect(sameFolder("D:/work/api/", "D:/work/api")).toBe(true);
    expect(applicable(store, "d:\\work\\api\\").map((m) => m.title)).toContain("API convention");
  });

  it("does not treat one folder as another that merely starts the same way", () => {
    expect(sameFolder("D:/work/api", "D:/work/api-v2")).toBe(false);
    expect(applicable(store, "D:/work/api-v2").map((m) => m.title)).not.toContain("API convention");
  });

  it("treats a project memory with no folder as global rather than unreachable", () => {
    const orphan = [memory("Older record", "Written before scopes existed.", { scope: "project" })];
    expect(applicable(orphan, here)).toHaveLength(1);
  });

  it("says nothing matches an empty folder on both sides", () => {
    expect(sameFolder("", "")).toBe(false);
    expect(sameFolder(null, undefined)).toBe(false);
  });
});

describe("refusing to write down a secret", () => {
  it("catches the key shapes that are unmistakable", () => {
    expect(looksSecret("the key is sk-abcdefghijklmnopqrstuvwxyz")).toBe(true);
    expect(looksSecret("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345")).toBe(true);
    expect(looksSecret("AKIAIOSFODNN7EXAMPLE")).toBe(true);
    expect(looksSecret("xoxb-1234567890-abcdefghij")).toBe(true);
    expect(looksSecret("-----BEGIN RSA PRIVATE KEY-----")).toBe(true);
    expect(looksSecret('api_key = "8f4b2c9e1a7d3f6b0c5e8a2d"')).toBe(true);
  });

  it("does not refuse ordinary sentences about secrets", () => {
    // Talking about where a key lives is exactly what a memory SHOULD record.
    expect(looksSecret("The API key is stored in the workspace secrets as MINIMAX_API_KEY.")).toBe(
      false
    );
    expect(looksSecret("Ask the user for their token; never read it from the repo.")).toBe(false);
  });

  it("refuses to store one, and refuses an empty memory", () => {
    expect(isStorable(memory("Key", "sk-abcdefghijklmnopqrstuvwxyz"))).toBe(false);
    expect(isStorable(memory("", "body"))).toBe(false);
    expect(isStorable(memory("title", "  "))).toBe(false);
    expect(isStorable(memory("Package manager", "The project uses pnpm."))).toBe(true);
  });
});

describe("staleness", () => {
  const now = Date.parse("2026-09-05T00:00:00Z");

  it("calls a memory stale once nothing has recalled it for three weeks", () => {
    expect(isStale({ lastUsedAt: "2026-08-01T00:00:00Z" }, { now })).toBe(true);
    expect(isStale({ lastUsedAt: "2026-09-01T00:00:00Z" }, { now })).toBe(false);
  });

  it("does not call a memory stale for never having been recalled", () => {
    // A memory written yesterday has no recall date, and reading that as three
    // weeks of neglect is how a brand new store shows up entirely greyed out.
    expect(isStale({}, { now })).toBe(false);
    expect(isStale({ lastUsedAt: "not a date" }, { now })).toBe(false);
  });
});

describe("not writing the same memory twice", () => {
  const store = [
    memory("Uses pnpm", "The project uses pnpm, not npm.", { scope: "global" }),
    memory("Allergic to peanuts", "Do not suggest peanut recipes.", { scope: "global" }),
    memory("API routes", "Handlers live in src/routes.", {
      scope: "project",
      folder: "D:/work/api",
    }),
  ];

  it("recognises the same fact written again", () => {
    const twin = findDuplicate(
      store,
      memory("Uses pnpm", "The project uses pnpm, not npm.", { scope: "global" })
    );
    expect(twin?.id).toBe("uses-pnpm");
  });

  it("treats a changed body under the same title as a correction, not a new fact", () => {
    // Keeping both is how a store starts contradicting itself.
    const twin = findDuplicate(
      store,
      memory("Uses pnpm", "They moved to bun in July.", { scope: "global" })
    );
    expect(twin?.id).toBe("uses-pnpm");
  });

  it("ignores case and punctuation in a title, as a person would", () => {
    expect(findDuplicate(store, memory("uses PNPM!", "Something.", { scope: "global" }))?.id).toBe(
      "uses-pnpm"
    );
  });

  it("does not merge two facts that are merely related", () => {
    // Both true. A wrong merge silently destroys one of them, which is far
    // worse than an untidy pair.
    const twin = findDuplicate(
      store,
      memory("Allergic to eggs", "Do not suggest egg recipes.", { scope: "global" })
    );
    expect(twin).toBe(null);
  });

  it("lets two projects hold the same sentence", () => {
    // In each one it is a fact about that project.
    const twin = findDuplicate(
      store,
      memory("API routes", "Handlers live in src/routes.", {
        scope: "project",
        folder: "D:/work/site",
      })
    );
    expect(twin).toBe(null);
  });

  it("does not merge a project memory into a global one", () => {
    const twin = findDuplicate(
      store,
      memory("Uses pnpm", "The project uses pnpm, not npm.", {
        scope: "project",
        folder: "D:/work/api",
      })
    );
    expect(twin).toBe(null);
  });

  it("scores plain text overlap the way the merge relies on", () => {
    expect(similarity("the project uses pnpm", "the project uses pnpm")).toBe(1);
    expect(similarity("allergic to eggs", "allergic to peanuts")).toBeLessThan(0.5);
    expect(similarity("", "anything")).toBe(0);
  });
});

describe("choosing what is worth carrying into this message", () => {
  const store = [
    memory("Uses pnpm", "Not npm.", { useCount: 50 }),
    memory("Deploy branch", "Ships straight to master, never a feature branch.", { useCount: 1 }),
    memory("Coffee", "Black.", { useCount: 40 }),
    memory("Pinned rule", "Always applies.", { pinned: true, useCount: 0 }),
    memory("Where we left off", "Routes were moved.", { kind: "handover", useCount: 0 }),
  ];

  it("carries the memory that answers the question ahead of the popular ones", () => {
    // With no room for everything, the one about deploying wins over the two
    // that have merely been used more. This was a popularity contest before.
    // Room for the two that always go first, plus exactly one more.
    const chosen = forPrompt(store, { query: "which branch do we deploy from", budget: 140 });
    const titles = chosen.map((m) => m.title);
    expect(titles).toContain("Deploy branch");
    expect(titles).not.toContain("Uses pnpm");
    expect(titles).not.toContain("Coffee");
  });

  it("always carries the pinned memories and the handover note first", () => {
    const chosen = forPrompt(store, { query: "deploy", budget: 5000 });
    expect(
      chosen
        .slice(0, 2)
        .map((m) => m.title)
        .sort()
    ).toEqual(["Pinned rule", "Where we left off"]);
  });

  it("falls back to what has proved useful when the message matches nothing", () => {
    // A small baseline for the case where the words do not overlap. Better than
    // an empty block, and the recall tool covers the rest.
    const chosen = forPrompt(store, { query: "kubernetes ingress", budget: 5000 });
    expect(chosen.map((m) => m.title)).toContain("Uses pnpm");
  });

  it("ranks the same way with no message at all", () => {
    const chosen = forPrompt(store, { budget: 5000 });
    expect(chosen[0].pinned || chosen[0].kind === "handover").toBe(true);
  });
});

describe("bringing an older memory up to date", () => {
  const previous = memory("Uses npm", "The project uses npm.", {
    tags: ["tooling", "build"],
    pinned: true,
    createdAt: "2026-01-01T00:00:00Z",
    useCount: 7,
  });

  it("takes the new wording", () => {
    expect(merged(previous, { title: "Uses pnpm", body: "Moved off npm." }).body).toBe(
      "Moved off npm."
    );
  });

  it("keeps everything the memory had accumulated", () => {
    // Saving the same fact again with no tags used to wipe the tags somebody
    // had added, so a memory got worse every time it was confirmed.
    const out = merged(previous, { title: "Uses pnpm", body: "Moved off npm.", tags: [] });
    expect(out.tags).toEqual(["tooling", "build"]);
    expect(out.pinned).toBe(true);
    expect(out.createdAt).toBe("2026-01-01T00:00:00Z");
    expect(out.useCount).toBe(7);
  });

  it("adds a genuinely new tag rather than replacing the old ones", () => {
    const out = merged(previous, { tags: ["packages"] });
    expect(out.tags).toEqual(["tooling", "build", "packages"]);
  });

  it("does not add the same tag twice in a different case", () => {
    expect(merged(previous, { tags: ["Tooling"] }).tags).toEqual(["tooling", "build"]);
  });

  it("pins when either side is pinned", () => {
    expect(merged({ pinned: false }, { pinned: true }).pinned).toBe(true);
    expect(merged({ pinned: true }, { pinned: false }).pinned).toBe(true);
  });
});
