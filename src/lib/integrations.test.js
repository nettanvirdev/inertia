import { describe, it, expect, afterEach } from "vitest";
import { isCatalogueFresh, forgetCatalogue, composio } from "./integrations";

/**
 * The rule that decides whether the Composio pane draws a list or a spinner.
 *
 * It is one comparison, and it is worth pinning down anyway: get it wrong in
 * one direction and the catalogue is refetched on every tab switch, which is
 * the bug this cache exists to fix; get it wrong in the other and a window left
 * open across a Composio release keeps showing a catalogue that no longer
 * matches what Refresh would return.
 */

const NOW = 1_700_000_000_000;
const HOUR = 60 * 60 * 1000;
const entry = (fetchedAt) => ({ items: [{ slug: "gmail" }], fetchedAt });

describe("catalogue freshness", () => {
  it("keeps a catalogue read moments ago", () => {
    expect(isCatalogueFresh(entry(NOW - 1000), NOW)).toBe(true);
  });

  it("keeps one read hours ago, because Composio does not ship hourly", () => {
    expect(isCatalogueFresh(entry(NOW - 5 * HOUR), NOW)).toBe(true);
  });

  it("gives up on one older than the ceiling", () => {
    expect(isCatalogueFresh(entry(NOW - 7 * HOUR), NOW)).toBe(false);
  });

  it("honours a ceiling the caller names", () => {
    expect(isCatalogueFresh(entry(NOW - 90 * 1000), NOW, 60 * 1000)).toBe(false);
  });

  // A clock that has gone backwards should cost one refetch, not lock the
  // window to a catalogue whose age it can never compute.
  it("treats a timestamp from the future as stale", () => {
    expect(isCatalogueFresh(entry(NOW + HOUR), NOW)).toBe(false);
  });

  it("refuses anything that is not a catalogue", () => {
    expect(isCatalogueFresh(null, NOW)).toBe(false);
    expect(isCatalogueFresh({ fetchedAt: NOW }, NOW)).toBe(false);
    expect(isCatalogueFresh({ items: [] }, NOW)).toBe(false);
  });

  // An empty catalogue is a real answer - a key with no project behind it - and
  // refetching it on every mount would be the original bug wearing a hat.
  it("keeps an empty but freshly read catalogue", () => {
    expect(isCatalogueFresh({ items: [], fetchedAt: NOW - 1000 }, NOW)).toBe(true);
  });
});

/**
 * The connected apps, held for the life of the window.
 *
 * They are workspace files and cost milliseconds to read, but a millisecond is
 * a frame with nothing on it - and what that looks like is the list of your
 * connected apps blinking out and coming back every time you open the screen.
 */
describe("the connections cache", () => {
  afterEach(() => {
    forgetCatalogue();
    delete globalThis.window;
  });

  it("has nothing to offer before the first read", () => {
    expect(composio.cachedConnections()).toBeNull();
  });

  it("hands back what the last read returned, without asking again", async () => {
    const rows = [{ id: "c1", toolkitSlug: "gmail" }];
    let calls = 0;
    globalThis.window = {
      composioAPI: {
        connections: async () => {
          calls += 1;
          return { ok: true, data: rows };
        },
      },
    };

    expect(await composio.connections()).toEqual(rows);
    expect(composio.cachedConnections()).toEqual(rows);
    expect(calls).toBe(1);
  });
});
