import { llmClient } from "@/lib/llm";

/**
 * The pane's view of the model bridge.
 *
 * `llmClient` rejects on failure, and every one of its failures is already a
 * sentence a person can act on - the backend names the status, the host
 * that did not resolve, the port nothing is listening on. Turning that into an
 * `{ ok }` result here rather than letting it throw means a dead endpoint
 * renders as a line of text next to the button that asked, instead of taking
 * the dialog down mid-edit. A form has to survive the thing it is testing
 * being broken, because that is the whole reason someone is in it.
 */

async function attempt(work) {
  try {
    return { ok: true, value: await work() };
  } catch (failure) {
    return { ok: false, error: failure?.message || "The request failed.", status: failure?.status };
  }
}

/** `GET /models`, as a list of `{ id, label }` the picker can render. */
export async function listModels(provider) {
  const result = await attempt(() => llmClient.listModels(provider));
  if (!result.ok) return result;
  // The bridge answers with the array; a shape carrying it under `models` is
  // accepted too so this does not break if the envelope ever comes back.
  const rows = Array.isArray(result.value) ? result.value : (result.value?.models ?? []);
  return {
    ok: true,
    models: rows.map((row) =>
      typeof row === "string" ? { id: row, label: "" } : { id: row.id, label: row.label ?? "" }
    ),
  };
}

/** One cheap round trip: does it resolve, speak the protocol, accept the key. */
export async function testProvider(provider) {
  const result = await attempt(() => llmClient.testProvider(provider));
  if (!result.ok) return result;
  const data = result.value ?? {};
  return {
    ok: true,
    models: Number(data.models ?? 0),
    latencyMs: Number(data.latencyMs ?? 0),
    note: typeof data.note === "string" ? data.note : "",
    protocol: typeof data.protocol === "string" ? data.protocol : "",
    detected: Boolean(data.detected),
  };
}

/**
 * The sentence to show for a failure.
 *
 * Almost always the message that came back, because it was written to be read.
 * The status mapping is only a floor for the day something rejects without one.
 */
export function describeFailure(result) {
  if (result?.error) return result.error;
  const status = result?.status;
  if (status === 401 || status === 403) {
    return "The key was rejected. Check the API key, or whether this endpoint wants one at all.";
  }
  if (status === 404) {
    return "Nothing answered at that base URL. Some gateways are not mounted at /v1 - check the path.";
  }
  if (status === 429) return "The provider is rate limiting this key right now. Try again shortly.";
  return "The request failed.";
}
