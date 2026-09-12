/**
 * The files a thread produced, gathered from the transcript itself.
 *
 * Two things count as a file here and they are not the same kind of thing:
 *
 *   · an ATTACHMENT is a real artifact handed over in a message - it has a
 *     size, and the bytes themselves travel with the row (`dataUrl` for an
 *     image, `text` for anything readable), so it can be shown and saved
 *     without going back to a store;
 *   · a PATH is something a `files` tool call touched on the computer. There is
 *     no blob to download, only a location, so the row says who wrote it and
 *     when rather than pretending to a byte count.
 *
 * Keeping both in one list is deliberate: "what came out of this conversation"
 * is the question the panel answers, and the answer is not only uploads.
 *
 * This is a pure function over transcript blocks rather than a lookup by id, so
 * it works against the store's live messages and will keep working when those
 * arrive from a backend instead of a fixture.
 */

const EXT_KIND = {
  md: "markdown",
  markdown: "markdown",
  json: "json",
  yaml: "config",
  yml: "config",
  toml: "config",
  cjs: "code",
  mjs: "code",
  js: "code",
  jsx: "code",
  ts: "code",
  tsx: "code",
  py: "code",
  sh: "code",
  css: "code",
  html: "code",
  png: "image",
  jpg: "image",
  jpeg: "image",
  gif: "image",
  webp: "image",
  svg: "image",
  pdf: "pdf",
  csv: "table",
  tsv: "table",
  xlsx: "table",
  txt: "text",
  log: "text",
};

/** The icon each kind spends, by name - resolved through `<Icon name>`. */
export const FILE_KIND_ICON = {
  markdown: "FileText",
  json: "FileJson",
  config: "SlidersHorizontal",
  code: "Code2",
  image: "Image",
  pdf: "File",
  table: "Table",
  text: "FileText",
  folder: "Folder",
  file: "File",
};

export function fileKind(name) {
  if (!name) return "file";
  if (name.endsWith("/")) return "folder";
  const ext = name.split(".").pop()?.toLowerCase();
  return (ext && EXT_KIND[ext]) || "file";
}

/** 1 KB is 1024 B here, and one decimal is as much precision as a row can use. */
export function formatBytes(bytes) {
  if (bytes == null) return null;
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${Math.round(kb)} KB`;
  return `${(kb / 1024).toFixed(1)} MB`;
}

/**
 * Just the last segment, so a deep path still reads as a filename.
 *
 * Both separators. This app is Windows-first, its paths look like
 * `C:\Users\me\Projects\site\app\components\Header.tsx`, and splitting on
 * the forward slash alone handed the whole thing back - so the Files panel
 * showed seventeen rows that all read `C:\Users\me\Projects\site...`
 * and were impossible to tell apart. The one thing the row exists to say was
 * the one thing it truncated away.
 */
export function baseName(path) {
  const trimmed = String(path ?? "").replace(/[\\/]+$/, "");
  const cut = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return trimmed.slice(cut + 1) || String(path ?? "");
}

/**
 * @param {Array} blocks  transcript blocks for one thread
 * @returns {Array<{id, name, path, kind, sizeBytes, origin, agentId, at,
 *                  dataUrl?, text?}>}
 *          newest first; `origin` is "attached" or "written".
 */
/**
 * The file tools, and where each one keeps the path it touched.
 *
 * There is no single "files" tool - there is `write`, `edit` and `patch`, and
 * each names its target in `filePath`. `patch` can touch several files in one
 * diff and reports them in its result metadata; the single-file tools carry the
 * path in their arguments, which is there the instant the call starts rather
 * than only once it finishes.
 */
const WRITE_TOOLS = new Set(["write", "edit", "patch"]);

/**
 * The paths an agent deliberately handed over, which is a different claim from
 * the paths it touched. See `main/tools/builtin/present.cjs`.
 */
function presentedPaths(name, metadata) {
  if (name !== "present") return [];
  return (metadata?.presented ?? [])
    .map((one) => (typeof one === "string" ? one : one?.path))
    .filter((one) => typeof one === "string" && one.trim());
}

/** Every path a tool call touched, however that tool records them. */
function pathsFromTool(name, args, metadata) {
  const out = [];
  const push = (value) => {
    if (typeof value === "string" && value.trim()) out.push(value.trim());
  };
  if (!WRITE_TOOLS.has(name)) return out;

  // A single-file tool: the argument it was called with.
  push(args?.filePath ?? args?.path);
  // A patch can name several, in whichever shape the tool reported.
  for (const entry of metadata?.files ?? metadata?.paths ?? []) {
    push(typeof entry === "string" ? entry : entry?.path);
  }
  return out;
}

/**
 * One transcript entry - a message, which may be a reply with tool `parts`, or
 * a top-level tool block from a seeded thread - reduced to the files it touched
 * and the attachments it carried. Kept separate so both shapes go through the
 * same field names.
 */
function filesFromEntry(entry, add) {
  const at = entry.createdAt ?? entry.updatedAt ?? entry.at;

  // A top-level tool block.
  if (entry.type === "tool") {
    for (const path of presentedPaths(entry.name, entry.metadata)) {
      add({
        id: `${entry.id ?? entry.callId ?? entry.name}:${path}`,
        name: baseName(path),
        path,
        kind: fileKind(path),
        sizeBytes: null,
        origin: "presented",
        agentId: entry.agentId,
        at,
        key: path,
      });
    }
    for (const path of pathsFromTool(entry.name, entry.args, entry.metadata)) {
      add({
        id: `${entry.id ?? entry.callId ?? entry.name}:${path}`,
        name: baseName(path),
        path,
        kind: fileKind(path),
        sizeBytes: null,
        origin: "written",
        agentId: entry.agentId,
        at,
        key: path,
      });
    }
    return;
  }

  // A reply, whose tool calls live in its parts.
  for (const part of entry.parts ?? []) {
    if (part?.type !== "tool") continue;
    for (const path of presentedPaths(part.name, part.metadata)) {
      add({
        id: `${part.callId ?? entry.id}:${path}`,
        name: baseName(path),
        path,
        kind: fileKind(path),
        sizeBytes: null,
        origin: "presented",
        agentId: entry.agentId,
        at,
        key: path,
      });
    }
    for (const path of pathsFromTool(part.name, part.args, part.metadata)) {
      add({
        id: `${part.callId ?? entry.id}:${path}`,
        name: baseName(path),
        path,
        kind: fileKind(path),
        sizeBytes: null,
        origin: "written",
        agentId: entry.agentId,
        at,
        key: path,
      });
    }
  }

  // Attachments hang off the message itself, whichever kind it is.
  for (const att of entry.attachments ?? []) {
    add({
      // Keyed on the id rather than the name, because two different screenshots
      // are both called `image.png` and dropping the second loses a file the
      // user can see in the transcript.
      key: att.id ?? att.name,
      id: att.id ?? att.name,
      name: att.name,
      path: att.name,
      // The composer's `kind` is only ever "image" or "text", too coarse for a
      // row: a `.json` and a `.md` are both text and want different glyphs.
      kind: att.kind === "image" ? "image" : fileKind(att.name),
      // `size` is what the composer writes; `sizeBytes` is what older
      // transcripts on disk carry. Reading both beats a migration.
      sizeBytes: att.size ?? att.sizeBytes ?? null,
      origin: "attached",
      agentId: entry.role === "user" ? null : entry.agentId,
      at,
      ...(att.dataUrl ? { dataUrl: att.dataUrl } : {}),
      ...(att.text != null ? { text: att.text } : {}),
    });
  }
}

export function collectThreadFiles(blocks = []) {
  const files = [];
  const seen = new Set();
  const add = (file) => {
    // The same file, written twice in a thread, is one row showing the latest
    // touch - so a later write replaces the timestamp of an earlier one rather
    // than stacking a second identical line.
    const key = file.key ?? file.id;
    if (seen.has(key)) {
      const existing = files.find((f) => (f.key ?? f.id) === key);
      if (existing && String(file.at) > String(existing.at)) existing.at = file.at;
      // Presenting a file the turn also wrote is the stronger claim, and it can
      // arrive after the write it is about. Promoting rather than adding keeps
      // it one row, in the section that says it was handed over on purpose.
      if (existing && file.origin === "presented") existing.origin = "presented";
      return;
    }
    seen.add(key);
    files.push(file);
  };

  for (const block of blocks) {
    if (block) filesFromEntry(block, add);
  }

  return files
    .map(({ key, ...file }) => file)
    .sort((a, b) => String(b.at).localeCompare(String(a.at)));
}
