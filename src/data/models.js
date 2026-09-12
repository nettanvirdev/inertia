/**
 * Model catalogue for Inertia.
 * Demo data only  -  prices and context windows are plausible but fictional.
 */

export const PROVIDERS = [
  {
    id: "anthropic",
    name: "Anthropic",
    icon: "Sparkles",
    connected: true,
    keyMasked: "sk-ant-api03-••••••••••••7f2a",
  },
  {
    id: "openai",
    name: "OpenAI",
    icon: "Circle",
    connected: true,
    keyMasked: "sk-proj-••••••••••••b41d",
  },
  {
    id: "google",
    name: "Google AI",
    icon: "Diamond",
    connected: true,
    keyMasked: "AIza••••••••••••Qm9K",
  },

  {
    id: "ollama",
    name: "Ollama (local)",
    icon: "HardDrive",
    connected: true,
    keyMasked: null,
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    icon: "Route",
    connected: false,
    keyMasked: null,
  },
];

export const MODELS = [
  {
    id: "claude-sonnet-4-6",
    providerId: "anthropic",
    name: "Claude Sonnet 4.6",
    contextWindow: 200000,
    inputPrice: 3.0,
    outputPrice: 15.0,
    capabilities: ["vision", "tools", "reasoning"],
    recommended: true,
  },
  {
    id: "claude-opus-4-6",
    providerId: "anthropic",
    name: "Claude Opus 4.6",
    contextWindow: 200000,
    inputPrice: 15.0,
    outputPrice: 75.0,
    capabilities: ["vision", "tools", "reasoning"],
    recommended: true,
  },
  {
    id: "claude-haiku-4-2",
    providerId: "anthropic",
    name: "Claude Haiku 4.2",
    contextWindow: 200000,
    inputPrice: 0.8,
    outputPrice: 4.0,
    capabilities: ["vision", "tools"],
    recommended: false,
  },
  {
    id: "gpt-5-1",
    providerId: "openai",
    name: "GPT-5.1",
    contextWindow: 400000,
    inputPrice: 5.0,
    outputPrice: 20.0,
    capabilities: ["vision", "tools", "reasoning"],
    recommended: true,
  },
  {
    id: "gpt-5-1-mini",
    providerId: "openai",
    name: "GPT-5.1 mini",
    contextWindow: 200000,
    inputPrice: 0.5,
    outputPrice: 2.4,
    capabilities: ["vision", "tools"],
    recommended: false,
  },
  {
    id: "o5-reasoning",
    providerId: "openai",
    name: "o5-reasoning",
    contextWindow: 256000,
    inputPrice: 9.0,
    outputPrice: 45.0,
    capabilities: ["tools", "reasoning"],
    recommended: false,
  },
  {
    id: "gemini-3-pro",
    providerId: "google",
    name: "Gemini 3 Pro",
    contextWindow: 1000000,
    inputPrice: 2.5,
    outputPrice: 12.0,
    capabilities: ["vision", "tools", "reasoning"],
    recommended: true,
  },
  {
    id: "gemini-3-flash",
    providerId: "google",
    name: "Gemini 3 Flash",
    contextWindow: 1000000,
    inputPrice: 0.2,
    outputPrice: 0.9,
    capabilities: ["vision", "tools"],
    recommended: false,
  },

  {
    id: "llama-4-70b-local",
    providerId: "ollama",
    name: "Llama 4 70B (local)",
    contextWindow: 131072,
    inputPrice: 0,
    outputPrice: 0,
    capabilities: ["tools"],
    recommended: false,
  },
  {
    id: "qwen3-32b-local",
    providerId: "ollama",
    name: "Qwen3 32B (local)",
    contextWindow: 65536,
    inputPrice: 0,
    outputPrice: 0,
    capabilities: ["tools", "reasoning"],
    recommended: false,
  },
  {
    id: "or-auto",
    providerId: "openrouter",
    name: "OpenRouter Auto",
    contextWindow: 200000,
    inputPrice: 1.5,
    outputPrice: 7.0,
    capabilities: ["vision", "tools"],
    recommended: false,
  },
];

/** @returns {object|undefined} */
export function getModelById(id) {
  return MODELS.find((m) => m.id === id);
}
