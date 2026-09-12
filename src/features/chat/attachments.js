/**
 * Turning a file someone dropped on the composer into something a model can read.
 *
 * There are exactly two things a model can be handed, and the difference is not
 * cosmetic:
 *
 *   · an IMAGE goes as a content part, which only a model with eyes can use;
 *   · TEXT goes into the message, because a model reads a CSV the same way it
 *     reads a paragraph.
 *
 * Everything else - a zip, an mp4, a compiled binary - is neither, and the
 * honest answer is to say so at the moment of attaching rather than to send
 * something unreadable and let the model apologise for it two seconds later.
 *
 * Sizes are capped here rather than at the request, because the failure at the
 * request is a 400 from a provider about a payload, which tells the user
 * nothing they can act on. A cap here names the file and the limit.
 */

/** Roughly a 4MB image, which is larger than any screenshot and under every
 *  provider's per-image ceiling once base64 has grown it by a third. */
export const MAX_IMAGE_BYTES = 4 * 1024 * 1024;

/** Text is cheap to send and expensive to think about: this is ~50k tokens. */
export const MAX_TEXT_BYTES = 200 * 1024;

const IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

/** Extensions that are text whatever the browser claims the type is. Windows
 *  reports no type at all for most of these. */
const TEXT_EXT = new Set([
  "txt", "md", "markdown", "json", "jsonl", "yaml", "yml", "toml", "ini", "cfg", "conf", "env",
  "csv", "tsv", "log", "sql", "graphql", "proto",
  "js", "jsx", "mjs", "cjs", "ts", "tsx", "py", "rb", "go", "rs", "java", "kt", "swift",
  "c", "h", "cpp", "hpp", "cs", "php", "sh", "bash", "zsh", "ps1", "bat",
  "html", "htm", "css", "scss", "less", "svg", "xml", "vue", "svelte", "astro",
  "dockerfile", "gitignore", "editorconfig", "lock",
]);

const extensionOf = (name) => String(name ?? "").split(".").pop()?.toLowerCase() ?? "";

export function kindOf(file) {
  const type = String(file?.type ?? "");
  if (IMAGE_TYPES.has(type)) return "image";
  // An SVG is both, and text is the more useful of the two: a model can read
  // the markup, and several providers reject it as an image anyway.
  if (type.startsWith("text/") || TEXT_EXT.has(extensionOf(file?.name))) return "text";
  if (type === "application/json") return "text";
  if (type.startsWith("image/")) return "unsupported-image";
  return "unsupported";
}

const readAs = (file, how) =>
  new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("The file could not be read."));
    reader.onload = () => resolve(reader.result);
    if (how === "dataUrl") reader.readAsDataURL(file);
    else reader.readAsText(file);
  });

/**
 * One file, read.
 *
 * Never throws: the caller is a UI that wants to show what went wrong per file
 * rather than lose a whole selection to one bad member of it.
 */
export async function readAttachment(file) {
  const name = file?.name || "file";
  const kind = kindOf(file);

  if (kind === "unsupported-image") {
    return {
      ok: false,
      name,
      why: `${file.type} is an image format models cannot read. PNG, JPEG, GIF and WebP work.`,
    };
  }
  if (kind === "unsupported") {
    return {
      ok: false,
      name,
      why: "Only images and text files can be attached. Put anything else on a computer and let the agent open it there.",
    };
  }

  const cap = kind === "image" ? MAX_IMAGE_BYTES : MAX_TEXT_BYTES;
  if (file.size > cap) {
    return {
      ok: false,
      name,
      why: `${formatBytes(file.size)} is over the ${formatBytes(cap)} limit for ${kind === "image" ? "an image" : "a text file"}.`,
    };
  }

  try {
    const content = await readAs(file, kind === "image" ? "dataUrl" : "text");
    return {
      ok: true,
      attachment: {
        id: `att-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
        name,
        kind,
        size: file.size,
        ...(kind === "image" ? { dataUrl: String(content) } : { text: String(content) }),
      },
    };
  } catch (error) {
    return { ok: false, name, why: error?.message ?? "The file could not be read." };
  }
}

export function formatBytes(bytes) {
  const value = Number(bytes) || 0;
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * The attachments, folded into what actually gets sent.
 *
 * Text becomes part of the message rather than a separate turn, fenced and
 * named, because a model reading a file wants to know which file it is reading.
 * Images become content parts, which is the only shape any provider takes them
 * in.
 */
export function foldAttachments(text, attachments) {
  const list = attachments ?? [];
  const texts = list.filter((one) => one.kind === "text");
  const images = list.filter((one) => one.kind === "image" && one.dataUrl);

  const body = [
    text,
    ...texts.map((one) => `Attached file \`${one.name}\`:\n\n\`\`\`\n${one.text}\n\`\`\``),
  ]
    .filter((part) => part && String(part).trim())
    .join("\n\n");

  if (!images.length) return { content: body, parts: null };

  return {
    content: body,
    parts: [
      { type: "text", text: body || "Attached." },
      ...images.map((one) => ({ type: "image_url", image_url: { url: one.dataUrl } })),
    ],
  };
}
