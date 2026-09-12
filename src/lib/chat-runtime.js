import * as React from "react";
import { findModel, parseModelRef } from "@shared/providers";
import { useWorkspace } from "./workspace";
import { llmClient, runChat, isLlmAvailable } from "./llm";
import { TITLE_INSTRUCTION, titlePrompt, cleanTitle } from "@shared/title";

/**
 * The bit between a thread and a provider.
 *
 * The store owns messages and knows nothing about HTTP; the bridge owns HTTP
 * and knows nothing about threads. This is the seam: it resolves which model a
 * thread should use, turns a transcript into a request, and streams the reply
 * back into the store one batch at a time.
 *
 * Keeping it out of the store matters because the store is the file every
 * screen imports. A provider round trip living in there would make the whole
 * app depend on the shape of a chat completion.
 */

/** How much of a transcript to send. Enough to hold a conversation, bounded so
 *  a long thread does not silently start costing a fortune per message. */
const HISTORY_LIMIT = 40;

/** Where providers, the default model and the user's prices are kept. */
const DOC = "settings.models";

export function useChatRuntime() {
  const { client, configured } = useWorkspace();
  const [settings, setSettings] = React.useState({ providers: [], defaultModel: "", prices: {} });

  // Live streams by thread, so a stop button knows which one it means and a
  // second send into the same thread cannot leave the first one orphaned.
  const streams = React.useRef(new Map());

  const reload = React.useCallback(async () => {
    if (!configured) return;
    try {
      const doc = await client.readDocument(DOC, {});
      setSettings({
        providers: doc?.providers ?? [],
        defaultModel: doc?.defaultModel ?? "",
        // What the user says their models cost, per model id. Their numbers
        // beat the shipped table, which is the only way a figure stays true
        // for a local endpoint or a negotiated rate.
        prices: doc?.prices ?? {},
      });
    } catch {
      /* an unreadable settings file is a chat that falls back to preview */
    }
  }, [client, configured]);

  React.useEffect(() => {
    reload();
  }, [reload]);

  /**
   * The providers pane writes to the same document, and a user who has just
   * added a provider should see it in every picker at once - not after a
   * relaunch, which is what actually happened for as long as this listened
   * without main ever announcing a document write.
   *
   * Narrow now that the announcement names the document. It used to re-read
   * settings on every write of any kind, including each message a running turn
   * appended, which is a file read per record for a file that had not changed.
   * `payload.memory` is the browser stand-in, which notifies without saying
   * what moved; an unrecognised payload still reloads rather than being
   * assumed irrelevant.
   */
  React.useEffect(() => {
    if (typeof client.onChanged !== "function") return undefined;
    return client.onChanged((payload) => {
      if (payload?.collection) return;
      if (payload?.document && payload.document !== DOC) return;
      reload();
    });
  }, [client, reload]);

  React.useEffect(() => {
    const map = streams.current;
    return () => {
      for (const cancel of map.values()) cancel();
      map.clear();
      llmClient.cancelAll?.().catch(() => {});
    };
  }, []);

  /**
   * Which model answers.
   *
   * The agent's own choice first, the workspace default second. An agent that
   * names a model no configured provider offers is not an error worth blocking
   * on - the user may have removed the provider - so it falls through rather
   * than refusing to send.
   */
  const resolveModel = React.useCallback(
    (agent) => {
      const candidates = [agent?.model, settings.defaultModel].filter(Boolean);
      for (const ref of candidates) {
        const found = findModel(settings.providers, ref);
        if (found) return found;
      }
      return null;
    },
    [settings]
  );

  const ready = configured && isLlmAvailable() && settings.providers.length > 0;

  const cancel = React.useCallback((threadId) => {
    const stop = streams.current.get(threadId);
    if (!stop) return false;
    stop();
    streams.current.delete(threadId);
    return true;
  }, []);

  /**
   * Send a transcript and stream the reply.
   *
   * The callbacks are the store's: this never touches message state itself, so
   * there is exactly one place that decides what a message looks like.
   */
  const send = React.useCallback(
    ({ agent, history, systemPrompt, threadId, modelRef, onDelta, onDone, onError }) => {
      // What the composer is showing wins. The picker is a statement about
      // this message, and an agent default that quietly overrode it would make
      // the control a decoration.
      const model = (modelRef && findModel(settings.providers, modelRef)) || resolveModel(agent);
      if (!model) {
        onError?.(
          new Error(
            settings.providers.length
              ? "No model is selected. Choose one under Settings, Providers."
              : "No provider is configured yet. Add one under Settings, Providers."
          )
        );
        return () => {};
      }

      const { modelId } = parseModelRef(model.ref);
      const messages = [];
      if (systemPrompt?.trim()) messages.push({ role: "system", content: systemPrompt.trim() });
      for (const entry of history.slice(-HISTORY_LIMIT)) {
        if (!entry.content?.trim()) continue;
        messages.push({
          role: entry.role === "agent" || entry.role === "assistant" ? "assistant" : "user",
          content: entry.content,
        });
      }

      const stop = runChat({
        providerId: model.providerId,
        model: modelId,
        messages,
        onDelta,
        onDone: (event) => {
          streams.current.delete(threadId);
          onDone?.(event);
        },
        onError: (error) => {
          streams.current.delete(threadId);
          onError?.(error);
        },
      });

      streams.current.set(threadId, stop);
      return stop;
    },
    [resolveModel, settings.providers]
  );

  /**
   * Ask a model to name a conversation.
   *
   * One short request, no tools, no streaming into anything - the answer is a
   * handful of words that either becomes the thread's name or is thrown away.
   * It resolves to null rather than throwing for every way it can go wrong (no
   * provider, no key, a model that answered with a paragraph), because the
   * caller's fallback is the name the thread already has, and a failed rename
   * is not something to interrupt anybody about.
   */
  const nameConversation = React.useCallback(
    ({ agent, modelRef, text, signal }) => {
      const model = (modelRef && findModel(settings.providers, modelRef)) || resolveModel(agent);
      const body = String(text ?? "").trim();
      if (!model || !body) return Promise.resolve(null);

      const { modelId } = parseModelRef(model.ref);
      return new Promise((resolve) => {
        let answer = "";
        let settled = false;
        const done = (value) => {
          if (settled) return;
          settled = true;
          signal?.removeEventListener?.("abort", abort);
          resolve(value);
        };
        const stop = runChat({
          providerId: model.providerId,
          model: modelId,
          messages: [
            { role: "system", content: TITLE_INSTRUCTION },
            { role: "user", content: titlePrompt(body) },
          ],
          temperature: 0,
          // Enough for five words and, on a thinking model, the tokens it
          // spends before writing them. Not enough to pay for a paragraph.
          maxTokens: 256,
          onDelta: (delta) => {
            answer += delta;
          },
          onDone: () => done(cleanTitle(answer)),
          onError: () => done(null),
        });
        function abort() {
          stop();
          done(null);
        }
        signal?.addEventListener?.("abort", abort, { once: true });
      });
    },
    [resolveModel, settings.providers]
  );

  return {
    ready,
    settings,
    providers: settings.providers,
    resolveModel,
    send,
    cancel,
    reload,
    nameConversation,
  };
}
