import { normalizeBaseUrl } from "@shared/providers";

/**
 * The window's view of the model bridge.
 *
 * Same shape as the workspace client and for the same reason: one adapter
 * forwards to the desktop bridge, and a stand-in keeps the UI usable in a plain
 * browser tab. Without the stand-in every screen that touches a provider would
 * be unreachable outside the packaged app, which is exactly the UI you most
 * want to iterate on.
 *
 * The stand-in does not fake a model. It fails honestly, saying that talking to
 * a provider needs the desktop app, so nobody mistakes a preview for a working
 * connection.
 */

function unwrap(reply) {
  if (!reply || typeof reply !== "object") throw new Error("The app did not answer.");
  if (reply.ok) return reply.data;
  throw new Error(reply.error || "That request was refused.");
}

function bridgeClient(api) {
  return {
    kind: "bridge",
    async providers() {
      return unwrap(await api.providers());
    },
    async listModels(provider) {
      return unwrap(await api.models(provider));
    },
    async testProvider(provider) {
      return unwrap(await api.test(provider));
    },
    async chat(request) {
      return unwrap(await api.chat(request));
    },
    async cancel(id) {
      return unwrap(await api.cancel(id));
    },
    async cancelAll() {
      return unwrap(await api.cancelAll());
    },
    onEvent: api.onEvent,
  };
}

function previewClient() {
  const unavailable = (what) => {
    throw new Error(
      `${what} needs the desktop app. A browser cannot reach a provider directly - the request would be blocked by CORS, and the API key would have to live in the page.`
    );
  };

  return {
    kind: "preview",
    async providers() {
      return { providers: [], defaultModel: "" };
    },
    async listModels(provider) {
      return unavailable(`Fetching models from ${normalizeBaseUrl(provider?.baseUrl) || "a provider"}`);
    },
    async testProvider() {
      return unavailable("Testing a connection");
    },
    async chat() {
      return unavailable("Talking to a model");
    },
    async cancel() {
      return { cancelled: false };
    },
    async cancelAll() {
      return { cancelled: 0 };
    },
    onEvent() {
      return () => {};
    },
  };
}

let client;

export function llmClientFor() {
  if (!client) {
    const api = typeof window !== "undefined" ? window.llmAPI : null;
    client = api ? bridgeClient(api) : previewClient();
  }
  return client;
}

/** A getter that reads like the object it stands in for, so call sites stay flat. */
export const llmClient = new Proxy(
  {},
  {
    get(_target, key) {
      const real = llmClientFor();
      const value = real[key];
      return typeof value === "function" ? value.bind(real) : value;
    },
  }
);

export function isLlmAvailable() {
  return llmClientFor().kind === "bridge";
}

/**
 * Run one completion, collapsing the event stream into three callbacks.
 *
 * Text arrives token by token and the caller almost always wants to append it
 * to something, so this hands back the delta rather than the accumulated
 * string: accumulating here would mean every consumer re-renders on a value it
 * did not ask for. Returns a `cancel` that is safe to call at any point,
 * including before the stream id has come back.
 */
export function runChat({ providerId, model, messages, temperature, maxTokens, onDelta, onReasoning, onDone, onError }) {
  const api = llmClientFor();
  let streamId = null;
  let cancelled = false;
  let unsubscribe = () => {};

  const finish = (fn, ...args) => {
    unsubscribe();
    if (typeof fn === "function") fn(...args);
  };

  unsubscribe = api.onEvent((event) => {
    if (!streamId || event.id !== streamId) return;
    if (event.type === "delta") onDelta?.(event.text);
    else if (event.type === "reasoning") onReasoning?.(event.text);
    else if (event.type === "done") finish(onDone, event);
    else if (event.type === "error") finish(onError, new Error(event.message));
  });

  api
    .chat({ providerId, model, messages, temperature, maxTokens })
    .then((started) => {
      streamId = started.id;
      // The user can hit stop before the handler has answered, in which case
      // the id only exists now - so the cancel is replayed here rather than
      // being lost.
      if (cancelled) api.cancel(streamId).catch(() => {});
    })
    .catch((error) => finish(onError, error));

  return function cancel() {
    cancelled = true;
    if (streamId) api.cancel(streamId).catch(() => {});
    unsubscribe();
  };
}
