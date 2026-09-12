import { call, subscribe } from "./envelope";

/**
 * The model bridge.
 *
 * The three questions the settings screen asks - what is configured, what does
 * this endpoint offer, does it work - plus the streaming call that is not a
 * turn. They run in the backend rather than the window because a browser cannot
 * reach a provider directly: the request would be blocked by CORS, and the API
 * key would have to live in the page.
 *
 * `models` and `test` take a whole provider record, not an id. The dialog asks
 * them about a draft the user is still typing, which has no saved id to look
 * up.
 *
 * `chat` is the odd one: it answers with a stream id rather than a reply, and
 * the reply arrives on `llm:event` tagged with that id. That is what lets the
 * window draw tokens as they land and offer Stop, neither of which a
 * request-and-response could do - and `cancel` names a stream by the same id,
 * because the id is the only handle a window has on work it does not own.
 */
export function llmBridge() {
  return {
    providers: () => call("llm_providers"),
    models: (provider) => call("llm_models", { provider: provider ?? {} }),
    test: (provider) => call("llm_test", { provider: provider ?? {} }),

    /** One completion, no tools, no turn. What `/compact` is made of. */
    chat: (request) => call("llm_chat", { request: request ?? {} }),

    // `cancelled: false` for a stream that has already ended is the ordinary
    // answer, not a failure: the click and the last token cross on the bridge,
    // and `runChat` cancels a start that failed while it is cleaning up.
    cancel: (id) => call("llm_cancel", { id: String(id ?? "") }),
    cancelAll: () => call("llm_cancel_all"),

    onEvent: (callback) => subscribe("llm:event", callback),
  };
}
