import * as React from "react";
import { voice } from "@/lib/voice";

/**
 * Talking to the app.
 *
 * The microphone is captured with MediaRecorder and the recording is sent to
 * the backend in one piece when it stops. Deliberately not a streaming
 * websocket: a streaming transcript is worth its complexity when a person is
 * dictating a document, and this is a sentence or two into a composer, where
 * the whole utterance arrives about as fast as the first partial would have.
 *
 * The browser's own SpeechRecognition would be free and on-device, and it is
 * the right first choice on the web. It is not available here - the system
 * webview this app runs in (WebView2, on Windows) ships without the speech
 * service that backs it - so there is one path rather than two, and it is the
 * one that works.
 *
 * Two ways to use it. `hold` records until told to stop, which is what a
 * press-and-hold button wants. `endpoint` watches the signal and stops on its
 * own once the person has stopped talking, which is what a click-once button
 * wants. The endpoint detector is a plain RMS threshold over the time-domain
 * data: a real voice-activity model is a dependency and a download, and the
 * job here is only to notice a room that went quiet.
 */

/** Poll often enough to feel immediate, rarely enough to cost nothing. */
const TICK_MS = 80;

/**
 * How long to listen to the room before deciding what counts as quiet.
 *
 * A fixed threshold was the first thing tried and it does not work. Measured on
 * a real USB microphone in a silent room, the noise floor peaks around 0.045
 * RMS - above the 0.035 that was hardcoded here - so the room itself read as
 * speech, the quiet never arrived, and the recorder ran until its two minute
 * cap without ever sending anything. Every microphone has a different floor,
 * and the only honest way to know one is to listen to it.
 */
const CALIBRATE_MS = 400;

/** Speech has to beat the room by this much before it counts as speech. */
const SPEECH_OVER_FLOOR = 2.2;

/**
 * Quiet is nearer the floor than speech is, and the gap between the two is
 * deliberate: one threshold would flicker on every pause between words.
 */
const QUIET_OVER_FLOOR = 1.35;

/** However quiet the room, a whisper is not a sentence. */
const MIN_SPEECH_RMS = 0.02;

/** A single loud tick is a door closing. Speech lasts longer than that. */
const SPEECH_TICKS = 2;

/** How long the quiet has to last before we believe the sentence ended. */
const ENDPOINT_MS = 850;

/** Past this a "quick note" has become a recording, and the upload has a cap. */
const MAX_MS = 120000;

export function useDictation({ deviceId, language, onText, onError } = {}) {
  const [state, setState] = React.useState("idle"); // idle | listening | thinking
  const [level, setLevel] = React.useState(0);

  // Everything the teardown needs, in refs, because the stop path runs from a
  // timer and from an event handler as well as from React.
  const media = React.useRef(null);
  const stream = React.useRef(null);
  const context = React.useRef(null);
  const ticker = React.useRef(null);
  const chunks = React.useRef([]);
  const cancelled = React.useRef(false);
  /**
   * Bumped by every teardown, so a `getUserMedia` that is still in flight can
   * tell that the thing which asked for it has gone.
   *
   * Without this, a start immediately followed by an unmount - which is exactly
   * what StrictMode does to every component in development - resolves its
   * permission promise after the cleanup has already run, and then opens a
   * recorder and a microphone that nothing is left to stop.
   */
  const generation = React.useRef(0);
  const settings = React.useRef({ language, onText, onError });

  settings.current = { language, onText, onError };

  const supported =
    typeof window !== "undefined" &&
    typeof window.MediaRecorder !== "undefined" &&
    Boolean(navigator?.mediaDevices?.getUserMedia);

  const teardown = React.useCallback(() => {
    generation.current += 1;
    if (ticker.current) {
      window.clearInterval(ticker.current);
      ticker.current = null;
    }
    if (context.current && context.current.state !== "closed") {
      context.current.close().catch(() => {
        /* a context that will not close is not worth a second failure */
      });
    }
    context.current = null;
    // Releasing every track is what turns the operating system's recording
    // indicator off. Leaving it on after a two-second dictation is alarming.
    stream.current?.getTracks().forEach((track) => track.stop());
    stream.current = null;
    media.current = null;
    setLevel(0);
  }, []);

  React.useEffect(() => () => teardown(), [teardown]);

  const stop = React.useCallback(() => {
    if (media.current && media.current.state !== "inactive") media.current.stop();
  }, []);

  const cancel = React.useCallback(() => {
    cancelled.current = true;
    stop();
  }, [stop]);

  /**
   * Watch the level and stop when the talking does.
   *
   * Silence only counts once speech has been heard, or the detector would fire
   * during the half second between pressing the button and starting to speak.
   */
  const watchForSilence = React.useCallback(
    (source) => {
      const Ctor = window.AudioContext ?? window.webkitAudioContext;
      if (!Ctor) return; // no meter and no endpointing; the button still stops it
      const ctx = new Ctor();
      context.current = ctx;
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 2048;
      ctx.createMediaStreamSource(source).connect(analyser);

      const data = new Uint8Array(analyser.fftSize);
      let heardSpeech = false;
      let quietFor = 0;
      let loudTicks = 0;

      // The room, before anyone speaks into it.
      let floor = 0;
      let calibrating = CALIBRATE_MS;
      let calibrationPeak = 0;

      ticker.current = window.setInterval(() => {
        analyser.getByteTimeDomainData(data);
        let sum = 0;
        for (let i = 0; i < data.length; i += 1) {
          const centred = (data[i] - 128) / 128;
          sum += centred * centred;
        }
        const rms = Math.sqrt(sum / data.length);

        if (calibrating > 0) {
          calibrationPeak = Math.max(calibrationPeak, rms);
          calibrating -= TICK_MS;
          if (calibrating > 0) return;
          floor = calibrationPeak;
        }

        const speechAt = Math.max(floor * SPEECH_OVER_FLOOR, MIN_SPEECH_RMS);
        const quietAt = Math.max(floor * QUIET_OVER_FLOOR, MIN_SPEECH_RMS * 0.6);

        // The meter reads against the threshold it will actually be judged by,
        // so a full-looking bar means "this is being heard as speech" rather
        // than "this is loud in the abstract".
        setLevel(Math.min(rms / (speechAt * 1.6), 1));

        if (rms >= speechAt) {
          loudTicks += 1;
          if (loudTicks >= SPEECH_TICKS) heardSpeech = true;
          quietFor = 0;
          return;
        }
        loudTicks = 0;
        if (!heardSpeech || rms >= quietAt) return;
        quietFor += TICK_MS;
        if (quietFor >= ENDPOINT_MS) stop();
      }, TICK_MS);
    },
    [stop]
  );

  const start = React.useCallback(async () => {
    if (!supported) {
      settings.current.onError?.(new Error("This window cannot record audio."));
      return;
    }
    if (media.current) return;

    cancelled.current = false;
    chunks.current = [];
    const mine = generation.current;

    let source;
    try {
      source = await navigator.mediaDevices.getUserMedia({
        audio:
          deviceId && deviceId !== "default"
            ? { deviceId: { exact: deviceId }, echoCancellation: true, noiseSuppression: true }
            : { echoCancellation: true, noiseSuppression: true },
      });
    } catch (failure) {
      // A refusal and a missing device are different problems with the same
      // shape, and the browser's own message is more accurate than a guess.
      settings.current.onError?.(
        new Error(
          failure?.name === "NotAllowedError"
            ? "The microphone was not allowed. Grant access to it and try again."
            : `The microphone could not be opened. ${failure?.message ?? ""}`.trim()
        )
      );
      return;
    }

    // The permission dialog is slow enough that a component can be gone by the
    // time it is answered. Hand the device straight back rather than recording
    // for nobody.
    if (generation.current !== mine) {
      source.getTracks().forEach((track) => track.stop());
      return;
    }

    stream.current = source;
    // No options: the container Chromium picks for itself is the one it can
    // definitely produce, and ElevenLabs sniffs the container anyway.
    const recorder = new MediaRecorder(source);
    media.current = recorder;

    recorder.ondataavailable = (event) => {
      if (event.data?.size) chunks.current.push(event.data);
    };

    recorder.onstop = async () => {
      const parts = chunks.current;
      const mimeType = parts[0]?.type || recorder.mimeType || "audio/webm";
      chunks.current = [];
      teardown();

      if (cancelled.current) {
        setState("idle");
        return;
      }
      const blob = new Blob(parts, { type: mimeType });
      if (!blob.size) {
        setState("idle");
        settings.current.onError?.(new Error("Nothing was recorded."));
        return;
      }

      setState("thinking");
      try {
        const buffer = await blob.arrayBuffer();
        let binary = "";
        const bytes = new Uint8Array(buffer);
        // In chunks, because spreading a megabyte of bytes into apply()
        // overflows the argument list.
        for (let i = 0; i < bytes.length; i += 8192) {
          binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
        }
        const result = await voice.transcribe({
          bytes: btoa(binary),
          mimeType,
          language: settings.current.language || undefined,
        });
        if (result?.text) settings.current.onText?.(result.text);
        else settings.current.onError?.(new Error("Nothing was said, or nothing was heard."));
      } catch (failure) {
        settings.current.onError?.(failure);
      } finally {
        setState("idle");
      }
    };

    // A timeslice keeps the buffer flowing so a long recording is not one
    // allocation at the end, and it is what makes the cap below cheap to honour.
    recorder.start(250);
    setState("listening");
    watchForSilence(source);
    window.setTimeout(() => {
      if (media.current === recorder && recorder.state !== "inactive") recorder.stop();
    }, MAX_MS);
  }, [deviceId, supported, teardown, watchForSilence]);

  return { state, level, supported, start, stop, cancel };
}
