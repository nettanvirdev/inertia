/**
 * The Library's catalogue, derived from the conversations rather than stored
 * beside them.
 *
 * There used to be a hand-written roster here and a `useState` holding it, so
 * the screen showed artifacts nobody had made and lost a deletion on unmount.
 * The archive that actually exists is the transcript: every attachment is in a
 * message and every written path is in a tool call, both already gathered by
 * `features/chat/thread-files.js`. Reading from there means the Library and the
 * chat's Files panel can never disagree about what a thread produced, and an
 * artifact cannot outlive the message that carries it.
 *
 * The consequence worth stating: there is nothing to delete from the Library.
 * Removing an artifact would mean editing a transcript, which is the chat's
 * business, so this screen offers opening and saving and no illusion of a
 * catalogue it owns.
 *
 * Everything here is pure, so it is testable and so a future backend can hand
 * the same shapes in without any of it changing.
 */

import { collectThreadFiles } from "@/features/chat/thread-files";

/** `fileKind` is about which glyph a row gets; a Library tab is a coarser
 *  question, and these are the only tabs the screen has. */
const CATEGORY_BY_KIND = {
  image: "image",
  markdown: "document",
  text: "document",
  pdf: "document",
  json: "data",
  config: "data",
  table: "data",
  code: "code",
};

/** Media a `files` tool can name on a computer. There is no attachment kind for
 *  either - the composer only takes images and text - so these arrive as paths.
 *  One inside the Inertia folder is read back and played; one on some other
 *  disk cannot be, and the screen says where it is instead. */
const AUDIO_EXT = new Set(["mp3", "wav", "m4a", "ogg", "oga", "flac", "aac", "opus", "wma"]);
const VIDEO_EXT = new Set(["mp4", "mov", "webm", "mkv", "avi", "m4v", "mpg", "mpeg"]);

const MIME_BY_EXT = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  pdf: "application/pdf",
  json: "application/json",
  csv: "text/csv",
  tsv: "text/tab-separated-values",
  md: "text/markdown",
  markdown: "text/markdown",
  txt: "text/plain",
  log: "text/plain",
  html: "text/html",
  css: "text/css",
  js: "text/javascript",
  mjs: "text/javascript",
  cjs: "text/javascript",
  jsx: "text/javascript",
  ts: "text/typescript",
  tsx: "text/typescript",
  py: "text/x-python",
  sh: "text/x-shellscript",
  yaml: "text/yaml",
  yml: "text/yaml",
  toml: "text/plain",
  mp3: "audio/mpeg",
  wav: "audio/wav",
  m4a: "audio/mp4",
  ogg: "audio/ogg",
  flac: "audio/flac",
  mp4: "video/mp4",
  mov: "video/quicktime",
  webm: "video/webm",
  mkv: "video/x-matroska",
};

const extensionOf = (name) => String(name ?? "").split(".").pop()?.toLowerCase() ?? "";

/** Which Library tab an artifact belongs under. Unknown is `document` rather
 *  than a tab of its own: a file with no extension is still a thing you read. */
export function libraryCategory(name, kind) {
  const ext = extensionOf(name);
  if (AUDIO_EXT.has(ext)) return "audio";
  if (VIDEO_EXT.has(ext)) return "video";
  return CATEGORY_BY_KIND[kind] ?? CATEGORY_BY_KIND[ext] ?? "document";
}

/**
 * The type of an artifact, preferring what the bytes say over what the name does.
 *
 * A data URL carries its own type and a screenshot pasted out of a clipboard
 * arrives named `image.png` whatever it really is, so the prefix wins whenever
 * there is one.
 */
export function mimeFor({ name, dataUrl } = {}) {
  const declared = /^data:([^;,]+)[;,]/.exec(String(dataUrl ?? ""));
  if (declared) return declared[1];
  return MIME_BY_EXT[extensionOf(name)] ?? "application/octet-stream";
}

/** The base64 payload of a data URL, or null when it is not one. */
export function dataUrlPayload(dataUrl) {
  const comma = String(dataUrl ?? "").indexOf(",");
  if (!String(dataUrl ?? "").startsWith("data:") || comma < 0) return null;
  return String(dataUrl).slice(comma + 1);
}

/**
 * A name that can only ever land where it was aimed.
 *
 * Separators go, runs of dots collapse and a leading dot is dropped, because
 * the name comes out of a transcript: a model that wrote `../../.ssh/config`
 * into a tool call must not be able to steer a save out of the folder or into a
 * hidden file the user will never find again.
 */
export function safeFileName(name) {
  const cleaned = String(name ?? "")
    .replace(/[\\/:*?"<>|]/g, "-")
    .replace(/\.{2,}/g, ".")
    .replace(/^\.+/, "")
    .trim();
  return cleaned || "artifact";
}

/**
 * Where a saved copy lands inside the workspace.
 *
 * Under the thread rather than loose in `files/`, because two conversations
 * both producing `report.md` is the normal case and the second save must not
 * quietly replace the first.
 */
export function saveTargetPath(artifact) {
  const folder = safeFileName(artifact?.threadId ?? "chat");
  return `files/${folder}/${safeFileName(artifact?.name)}`;
}

/**
 * How a path artifact can be opened, which depends on where it is.
 *
 * `ws:reveal` resolves inside the workspace folder and throws on anything else,
 * while `agent:open-path` hands the string straight to the OS. An absolute path
 * is almost always a project the agent was working in rather than the Inertia
 * folder, so it goes to the OS; a relative one is only meaningful against some
 * working directory, and the workspace is the one this app can name.
 */
export function openTargetFor(path) {
  const target = String(path ?? "");
  if (!target) return { how: "unknown", target };
  const absolute = target.startsWith("/") || /^[a-zA-Z]:[\\/]/.test(target) || target.startsWith("\\\\");
  return { how: absolute ? "os" : "workspace", target };
}

/** True when the artifact carries its own bytes and can therefore be shown and
 *  saved without asking a computer for anything. */
export function hasContent(artifact) {
  return Boolean(artifact?.dataUrl || artifact?.text != null);
}

/** One row from `collectThreadFiles`, dressed as a Library record. */
export function toLibraryItem(file, thread) {
  const category = libraryCategory(file.name, file.kind);
  return {
    ...file,
    // Thread-scoped: a tool path row is only unique within its own transcript.
    id: `${thread?.id ?? "thread"}:${file.id}`,
    category,
    mime: mimeFor(file),
    createdAt: file.at ?? null,
    threadId: thread?.id ?? null,
    // A user's own attachment has no agent on it; the thread's agent is still
    // the right filter for it, because that is the conversation it belongs to.
    agentId: file.agentId ?? thread?.agentId ?? null,
    source: file.origin === "attached" ? "attachment" : "tool",
    summary:
      file.origin === "attached"
        ? `Attached in ${thread?.title ?? "a conversation"}.`
        : `Written to ${file.path}.`,
  };
}

/**
 * Every artifact in the workspace, newest first.
 *
 * @param {{threads?: Array, messages?: Record<string, Array>}} store
 */
export function collectLibraryArtifacts({ threads = [], messages = {} } = {}) {
  const items = [];
  for (const thread of threads) {
    for (const file of collectThreadFiles(messages[thread.id] ?? [])) {
      items.push(toLibraryItem(file, thread));
    }
  }
  return items.sort((a, b) => String(b.createdAt).localeCompare(String(a.createdAt)));
}
