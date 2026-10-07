import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * A third-party app's logo.
 *
 * The URL is never put in the `img` directly. This window's Content-Security
 * Policy is `img-src 'self' data: blob:` on purpose - it renders model-authored
 * markdown, and remote images are how a model would smuggle a conversation out
 * one pixel URL at a time. So the Rust backend fetches the bytes and hands back
 * a data URI, and the widening never has to happen.
 *
 * The white plate is the one hardcoded colour in this screen and it is
 * load-bearing: most vendor logos are transparent PNGs drawn in dark ink, so on
 * a dark surface they vanish entirely. The plate is white in BOTH themes for
 * that reason - a theme-aware plate would go dark exactly when the logo needs
 * it not to. `object-contain`, never `cover`, because vendors ship arbitrary
 * aspect ratios and cover crops them, which turns a wordmark into a slice of
 * one letter.
 *
 * When there is no logo, or it cannot be had, the first letter of the name sits
 * on a neutral plate instead. A broken-image glyph next to an app the user
 * connected reads as "this connection is broken", which it is not.
 */

/**
 * Resolved URLs, for the life of the window.
 *
 * The catalogue is a few hundred rows that mount and unmount as the user
 * scrolls and searches. Without this, every pass would be a fresh round trip
 * for a logo the window already decoded - and the flash of the letter plate
 * while it re-resolves is the visible half of that cost.
 */
const resolved = new Map();

/**
 * The same answers, settled, so a remount can read one without waiting.
 *
 * A promise that has already resolved still hands its value back a microtask
 * later, which is a render. On a catalogue of several hundred rows that is
 * several hundred extra renders and a flash of the letter plate on every one of
 * them as the list is scrolled - the exact stutter this screen was reported for.
 */
const settled = new Map();

function resolve(url) {
  if (!url) return Promise.resolve("");
  if (resolved.has(url)) return resolved.get(url);

  const bridge = window.composioAPI?.logo;
  const work = typeof bridge === "function"
    ? bridge(url)
        .then((reply) => (reply?.ok ? (reply.data ?? "") : ""))
        .catch(() => "")
        .then((value) => {
          settled.set(url, value);
          return value;
        })
    : Promise.resolve("");

  resolved.set(url, work);
  return work;
}

export const AppLogo = React.memo(function AppLogo({ src, name, className }) {
  // A data URI is already the answer, so it renders on the first frame rather
  // than after a round trip - which is what makes a bundled logo feel bundled.
  // A logo this window has already fetched is the same thing one step removed.
  const immediate =
    typeof src === "string" && src.startsWith("data:") ? src : (settled.get(src) ?? "");
  const [data, setData] = React.useState(immediate);

  React.useEffect(() => {
    if (immediate) {
      setData(immediate);
      return undefined;
    }

    let alive = true;
    setData("");
    resolve(src).then((value) => {
      if (alive) setData(value);
    });
    return () => {
      alive = false;
    };
  }, [src, immediate]);

  const letter = String(name ?? "").trim().charAt(0).toUpperCase() || "?";

  if (!data) {
    return (
      <span
        aria-hidden="true"
        className={cn(
          "grid shrink-0 place-items-center rounded-lg fill-secondary",
          "text-[11px] font-semibold text-muted-foreground",
          className
        )}
      >
        {letter}
      </span>
    );
  }

  return (
    <span
      className={cn("grid shrink-0 place-items-center overflow-hidden rounded-lg p-1", className)}
      style={{ backgroundColor: "#ffffff" }}
    >
      <img
        src={data}
        alt=""
        // Decoding off the main thread: a grid of logos decoded inline is a
        // frame the scroll does not get.
        decoding="async"
        onError={() => setData("")}
        className="size-full object-contain"
      />
    </span>
  );
});
