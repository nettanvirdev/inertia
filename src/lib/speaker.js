import { voice } from "./voice";

/**
 * Playing a reply out loud.
 *
 * Three decisions worth explaining, because the obvious implementation is
 * different from this one.
 *
 * First, this decodes bytes with the Web Audio API instead of handing a blob
 * URL to an `<audio>` element. That began as a constraint - the policy this
 * was written under had no `media-src`, so `<audio src="blob:...">` was
 * blocked - and the window's policy now allows `blob:` media, so it is a
 * choice. It stays because it needs nothing more: `decodeAudioData` takes the
 * ArrayBuffer that already crossed the IPC bridge, so no URL is minted, none
 * has to be revoked, and no resource is ever fetched by the page. The speaking
 * rate comes free with it: an AudioBufferSourceNode has a playbackRate, so
 * changing the slider costs nothing and does not re-synthesise a single
 * character.
 *
 * Second, it pipelines. A long reply is many utterances, and synthesising all
 * of them before saying the first word means several seconds of silence. One
 * utterance is rendered while the previous one plays, so speech starts after
 * the first short sentence rather than after the whole answer.
 *
 * Third, there is one of it. A call, an automatically-read reply and a voice
 * preview in settings are three screens that can all be alive at once, and
 * three speakers would talk over each other through the same pair of speakers.
 * The singleton at the bottom of this file is what every screen uses;
 * `createSpeaker` stays exported for tests, which want an instance they own.
 */

/** Base64 over the bridge is the only encoding that survives it intact. */
function toArrayBuffer(base64) {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

export function createSpeaker() {
  let context = null;
  let source = null;
  /** Bumped on every stop, so a render that is still in flight knows it lost. */
  let generation = 0;
  let state = { status: "idle", caption: "", id: null };
  const listeners = new Set();

  function emit(patch) {
    state = { ...state, ...patch };
    for (const listener of listeners) listener(state);
  }

  function audio() {
    if (!context || context.state === "closed") {
      const Ctor = window.AudioContext ?? window.webkitAudioContext;
      if (!Ctor) throw new Error("This window cannot play audio.");
      context = new Ctor();
    }
    return context;
  }

  function stop() {
    generation += 1;
    if (source) {
      // onended fires on an explicit stop too, and a handler that resolves the
      // play promise twice would advance the queue past an utterance.
      source.onended = null;
      try {
        source.stop();
      } catch {
        /* already finished; nothing to stop */
      }
      source = null;
    }
    if (state.status !== "idle") emit({ status: "idle", caption: "", id: null });
  }

  function play(buffer, rate) {
    return new Promise((resolve, reject) => {
      let node;
      try {
        const ctx = audio();
        node = ctx.createBufferSource();
        node.buffer = buffer;
        node.playbackRate.value = Number.isFinite(rate) && rate > 0 ? rate : 1;
        node.connect(ctx.destination);
      } catch (failure) {
        reject(failure);
        return;
      }
      source = node;
      node.onended = () => {
        if (source === node) source = null;
        resolve();
      };
      node.start();
    });
  }

  return {
    get state() {
      return state;
    },

    /** Fires immediately with the current state, then on every change. */
    subscribe(listener) {
      listeners.add(listener);
      listener(state);
      return () => listeners.delete(listener);
    },

    /**
     * Say a list of utterances in order.
     *
     * Resolves when the last one finishes, or immediately when a `stop` landed
     * mid-flight. It does not reject on a cancel: a person pressing stop is not
     * an error, and every caller would have to filter it out again.
     *
     * `id` is whatever the caller wants to recognise this run by - a message id,
     * usually - so a Speak button can tell "I am the one playing" from "someone
     * else is".
     */
    async speak(utterances, { voiceId, modelId, rate = 1, id = null, onUtterance } = {}) {
      const lines = (Array.isArray(utterances) ? utterances : [utterances])
        .map((line) => String(line ?? "").trim())
        .filter(Boolean);
      if (!lines.length) return;

      stop();
      const mine = generation;
      emit({ status: "loading", caption: "", id });

      const render = async (text) => {
        const clip = await voice.speak({ text, voiceId, modelId });
        return audio().decodeAudioData(toArrayBuffer(clip.bytes));
      };

      try {
        // One ahead, no further: rendering the whole reply up front is the
        // silence this exists to avoid, and rendering three ahead spends the
        // quota on sentences a person may well interrupt.
        let next = render(lines[0]);
        for (let i = 0; i < lines.length; i += 1) {
          const current = next;
          next = i + 1 < lines.length ? render(lines[i + 1]) : null;

          const buffer = await current;
          if (generation !== mine) return;

          emit({ status: "speaking", caption: lines[i], id });
          onUtterance?.(lines[i], i);
          await play(buffer, rate);
          if (generation !== mine) return;
        }
      } finally {
        if (generation === mine) emit({ status: "idle", caption: "", id: null });
      }
    },

    stop,

    /** Let go of the audio hardware. Called when the app closes. */
    dispose() {
      stop();
      listeners.clear();
      if (context && context.state !== "closed") {
        context.close().catch(() => {
          /* a context that will not close is not worth a second failure */
        });
      }
      context = null;
    },
  };
}

/** The one every screen shares. See the third note at the top of this file. */
export const speaker = createSpeaker();
