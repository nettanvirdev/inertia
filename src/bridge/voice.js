import { call } from "./envelope";

/**
 * The voice bridge: ElevenLabs, over the backend.
 *
 * Audio crosses as base64 in both directions rather than as a URL or a Blob.
 * The window's content policy allows no remote origin, so a URL would be
 * unfetchable; a string survives the bridge intact and the renderer decodes it
 * once with the Web Audio API.
 *
 * The key is never in this file, this window, or this process. Every call here
 * is a message to the backend, which resolves the secret by name and owns the
 * only copy.
 */
export function voiceBridge() {
  return {
    /** Whether a key is stored. Never throws: no workspace is simply "no". */
    configured: () => call("voice_configured"),

    /** Proves the key works rather than that a string is stored. */
    test: () => call("voice_test"),

    voices: () => call("voice_voices"),
    models: () => call("voice_models"),

    /** Text in, `{ mimeType, bytes, size }` out, where bytes is base64 mp3. */
    speak: (request) => call("voice_speak", { request: request ?? {} }),

    /** Base64 audio in, `{ text, language }` out. */
    transcribe: (request) => call("voice_transcribe", { request: request ?? {} }),
  };
}
