/**
 * Providers and model references.
 *
 * Inertia talks to one protocol by default: the OpenAI chat-completions API.
 * Not because it is the best design, but because it is the one everything
 * speaks - OpenAI, Groq, Together, OpenRouter, Ollama, vLLM,
 * llama.cpp and every gateway in between. One client, and the user's own
 * endpoint is a first-class citizen rather than a special case bolted onto a
 * list of blessed vendors.
 *
 * There is one exception, and it is `kind`. Anthropic's own API is spoken
 * natively because reaching Claude through an OpenAI-shaped gateway silently
 * gives up prompt caching, and for an agent that resends the whole conversation
 * at every step of a turn, prompt caching is most of the bill. `kind` is left
 * empty on every record anyone has already saved, and an empty one is decided
 * by the host - so nothing that worked before this field existed changes.
 *
 * So a provider is a name, a base URL, and a key. That is the whole model, and
 * it is deliberately the user's to name: two OpenAI-compatible endpoints from
 * the same vendor, or a local one and a hosted one, have to be tellable apart
 * by something the user chose.
 *
 * Both halves need it - the settings pane edits these, and the backend turns
 * them into requests.
 */

/** The reference that travels with a message: `<providerId>/<modelId>`. */
export function modelRef(providerId, modelId) {
  if (!providerId || !modelId) return "";
  return `${providerId}/${modelId}`;
}

/**
 * Split a reference back apart.
 *
 * The provider id never contains a slash and a model id very often does
 * (`meta-llama/Llama-3.3-70B`, `anthropic/claude-sonnet-4`), so the split is at
 * the FIRST slash and everything after it is the model.
 */
export function parseModelRef(ref) {
  const value = String(ref ?? "");
  const cut = value.indexOf("/");
  if (cut === -1) return { providerId: "", modelId: value };
  return { providerId: value.slice(0, cut), modelId: value.slice(cut + 1) };
}

/** A provider as stored. Everything optional has a sane meaning when absent. */
export function blankProvider() {
  return {
    id: "",
    name: "",
    baseUrl: "",
    // The NAME of a stored secret, never the value - so a settings file can be
    // exported, synced or shown in a screenshot without leaking a key.
    apiKeySecret: "",
    // Empty means "work it out from the base URL", which is what every record
    // saved before this field existed says.
    kind: "",
    headers: {},
    models: [],
    enabled: true,
  };
}

/**
 * Normalise a base URL to the point a request can be built from it.
 *
 * People paste all of these, and all of them should work:
 *   https://api.openai.com
 *   https://api.openai.com/
 *   https://api.openai.com/v1
 *   http://localhost:11434/v1/
 *
 * The rule: strip trailing slashes, and append `/v1` only when the URL has no
 * path of its own. A gateway mounted at `/openai` or `/api/v1` is not wrong,
 * and guessing a `/v1` onto it would break it.
 */
export function normalizeBaseUrl(input) {
  const raw = String(input ?? "").trim();
  if (!raw) return "";
  const withScheme = /^https?:\/\//i.test(raw) ? raw : `https://${raw}`;
  let url;
  try {
    url = new URL(withScheme);
  } catch {
    return withScheme.replace(/\/+$/, "");
  }
  const path = url.pathname.replace(/\/+$/, "");
  return `${url.origin}${path || "/v1"}`;
}

/**
 * Which wire protocol a provider speaks.
 *
 * Shared rather than living in the client, because the settings form has to
 * show the same answer the request will use. A form that says "OpenAI" beside
 * an endpoint the backend is about to address as Anthropic is a bug the
 * user can see and cannot explain.
 *
 * An empty `kind` is decided by the host, and only Anthropic's own hosts are
 * decided that way. A gateway that merely proxies Claude - OpenRouter, most
 * corporate ones - serves it over the other protocol, and guessing from the
 * model name instead of the host would break every one of those.
 */
export const PROTOCOLS = ["openai", "anthropic"];

const NATIVE_ANTHROPIC = /(^|\.)anthropic\.com$/i;

/**
 * A gateway that mounts its Anthropic-compatible routes under `/anthropic`
 * - MiniMax, and most that offer both - says so in the path. A path segment
 * exactly that, and only that: `/anthropic-proxy` or `/v1/anthropic-models`
 * say nothing.
 */
const ANTHROPIC_PATH = /(^|\/)anthropic(\/|$)/i;

export function protocolOf(provider) {
  const declared = String(provider?.kind ?? "").toLowerCase();
  if (PROTOCOLS.includes(declared)) return declared;
  try {
    const raw = String(provider?.baseUrl ?? "").trim();
    const withScheme = /^https?:\/\//i.test(raw) ? raw : `https://${raw}`;
    const url = new URL(withScheme);
    return NATIVE_ANTHROPIC.test(url.hostname) || ANTHROPIC_PATH.test(url.pathname)
      ? "anthropic"
      : "openai";
  } catch {
    return "openai";
  }
}

/** How the protocol is named on screen, and what choosing it actually buys. */
export const PROTOCOL_LABELS = {
  openai: "OpenAI-compatible",
  anthropic: "Anthropic (native)",
};

export function endpoint(provider, route) {
  const base = normalizeBaseUrl(provider?.baseUrl);
  if (!base) return "";
  return `${base}/${String(route).replace(/^\/+/, "")}`;
}

/** Problems worth blocking a save for, in the order a form should show them. */
export function validateProvider(provider, existing = []) {
  const errors = {};
  const name = String(provider?.name ?? "").trim();
  const baseUrl = String(provider?.baseUrl ?? "").trim();

  if (!name) errors.name = "Give this provider a name.";
  else if (
    existing.some((p) => p.id !== provider.id && p.name.trim() === name)
  ) {
    errors.name = "Another provider already has that name.";
  }

  if (!baseUrl) errors.baseUrl = "A base URL is required.";
  else {
    try {
      const url = new URL(
        /^https?:\/\//i.test(baseUrl) ? baseUrl : `https://${baseUrl}`,
      );
      // A key sent to a plaintext endpoint is a key sent in the clear. Local
      // addresses are the honest exception - that traffic never leaves the box.
      const local = /^(localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0)$/i.test(
        url.hostname,
      );
      if (url.protocol === "http:" && !local) {
        errors.baseUrl = "Use https, or the key travels in the clear.";
      }
    } catch {
      errors.baseUrl = "That is not a URL.";
    }
  }

  return errors;
}

/** A stable, readable id derived from the name the user chose. */
export function providerId(name, taken = []) {
  const base =
    String(name ?? "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 40) || "provider";
  if (!taken.includes(base)) return base;
  for (let n = 2; n < 200; n += 1) {
    if (!taken.includes(`${base}-${n}`)) return `${base}-${n}`;
  }
  return `${base}-${Date.now()}`;
}

/** Every usable model across every enabled provider, for a model picker. */
export function allModels(providers) {
  const out = [];
  for (const provider of providers ?? []) {
    if (provider.enabled === false) continue;
    for (const model of provider.models ?? []) {
      const id = typeof model === "string" ? model : model.id;
      if (!id) continue;
      const detail = typeof model === "string" ? {} : model;
      out.push({
        ref: modelRef(provider.id, id),
        id,
        label: detail.label || id,
        providerId: provider.id,
        providerName: provider.name,
        // Which dial this model has - a thinking budget or an effort level -
        // depends on the wire protocol as much as the id, so it travels.
        protocol: protocolOf(provider),
        // What the user said this model costs, per million tokens. Kept on the
        // model so the price is entered where the model is, and read back into
        // the cost the session panel shows. Undefined when they left it blank.
        ...(Number.isFinite(Number(detail.input)) ? { input: Number(detail.input) } : {}),
        ...(Number.isFinite(Number(detail.output)) ? { output: Number(detail.output) } : {}),
        ...(Number.isFinite(Number(detail.cachedInput)) ? { cachedInput: Number(detail.cachedInput) } : {}),
        ...(Number.isFinite(Number(detail.cacheWrite)) ? { cacheWrite: Number(detail.cacheWrite) } : {}),
        ...(Number.isFinite(Number(detail.context)) ? { context: Number(detail.context) } : {}),
      });
    }
  }
  return out;
}

export function findModel(providers, ref) {
  return allModels(providers).find((m) => m.ref === ref) ?? null;
}

/**
 * Well-known endpoints, offered as a starting point rather than a menu.
 *
 * The list exists so the common cases are one click and a key, and so the
 * shape of the field is obvious for the ones that are not on it. Anything
 * OpenAI-compatible works whether or not it appears here.
 */
export const PRESETS = [
  {
    name: "Anthropic",
    baseUrl: "https://api.anthropic.com/v1",
    secret: "ANTHROPIC_API_KEY",
    kind: "anthropic",
    icon: "Brain",
  },
  {
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    secret: "OPENAI_API_KEY",
    icon: "Circle",
  },
  {
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    secret: "OPENROUTER_API_KEY",
    icon: "Route",
  },
  {
    name: "Groq",
    baseUrl: "https://api.groq.com/openai/v1",
    secret: "GROQ_API_KEY",
    icon: "Zap",
  },
  {
    name: "Together",
    baseUrl: "https://api.together.xyz/v1",
    secret: "TOGETHER_API_KEY",
    icon: "Layers",
  },
  {
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
    secret: "DEEPSEEK_API_KEY",
    icon: "Waves",
  },
  {
    name: "xAI",
    baseUrl: "https://api.x.ai/v1",
    secret: "XAI_API_KEY",
    icon: "Sparkles",
  },
  {
    name: "Ollama",
    baseUrl: "http://localhost:11434/v1",
    secret: "",
    icon: "HardDrive",
  },
];
