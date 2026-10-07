/**
 * The window's view of the voice bridge.
 *
 * Same shape as `lib/integrations.js` and for the same reason: one adapter
 * unwraps the `{ ok, data }` envelope into a value or a thrown Error, and a
 * stand-in keeps the settings pane renderable in a plain browser tab, which is
 * where this UI actually gets iterated on.
 *
 * The stand-in never pretends to be configured. Reads degrade to empty lists so
 * a screen draws its empty state instead of an error wall; anything that would
 * spend a character of the user's quota says plainly that it needs the desktop
 * app. The key is never in this file, this window, or this process - every call
 * here is a message to the backend, which owns the only copy.
 */

function unwrap(reply) {
  if (!reply || typeof reply !== "object") throw new Error("The app did not answer.");
  if (reply.ok) return reply.data;
  throw new Error(reply.error || "That request was refused.");
}

function bridge() {
  return typeof window === "undefined" ? null : (window.voiceAPI ?? null);
}

/** True inside the desktop app, false in a browser tab. */
export function isDesktop() {
  return Boolean(bridge());
}

function unavailable(what) {
  throw new Error(
    `${what} needs the desktop app. A browser tab has no backend to make the request in, and the API key it needs is deliberately not reachable from the page.`
  );
}

export const voice = {
  /** Whether a key is stored. Never throws: no workspace is simply "no". */
  configured: () => bridge()?.configured().then(unwrap) ?? Promise.resolve(false),

  /** Proves the key works rather than that a string is stored. */
  test: () => bridge()?.test().then(unwrap) ?? unavailable("Testing the key"),

  voices: () => bridge()?.voices().then(unwrap) ?? Promise.resolve([]),
  models: () => bridge()?.models().then(unwrap) ?? Promise.resolve([]),

  /** Text in, `{ mimeType, bytes, size }` out, where bytes is base64 mp3. */
  speak: (request) => bridge()?.speak(request).then(unwrap) ?? unavailable("Speaking"),

  /** Base64 audio in, `{ text, language }` out. */
  transcribe: (request) =>
    bridge()?.transcribe(request).then(unwrap) ?? unavailable("Transcribing"),
};

/**
 * The stored voice settings, and their defaults.
 *
 * Kept here rather than in the pane because three different places read them:
 * the settings pane that edits them, the composer that dictates with them, and
 * the thread view that decides whether to read a reply aloud. A default that
 * lives in one component is a default the other two get wrong.
 */
export const VOICE_DEFAULTS = {
  /** Empty until the user picks one; there is no voice we can assume an account has. */
  voiceId: "",
  modelId: "eleven_flash_v2_5",
  rate: 1,
  autoSpeak: false,
  inputDeviceId: "default",
  outputDeviceId: "default",
  /** Empty means "work it out from the audio", which Scribe is good at. */
  language: "",
};

export const VOICE_DOCUMENT = "settings.voice";
