/**
 * Reading the shape of a result nobody wrote a card for.
 *
 * Every built-in tool has a body designed for it - the diff for an edit, the
 * command line for a shell, the picture for a read. An MCP server's tools have
 * nothing, because nobody here knows what they are: a search, a calendar, a
 * database, an image generator, all arriving through one door and all answering
 * with a blob of JSON on one very long line. The card showed it as exactly that,
 * a monospace string running off the right edge, which is the same as showing
 * nothing.
 *
 * What every one of them does have is structure. A list of results is a list
 * whatever it is a list of; a `url` is a link; a field called `image_url` ending
 * in `.png` is a picture. So the renderer is driven by the shape rather than by
 * the server: classify the value, name the fields by the role their key implies,
 * and draw the thing that shape deserves.
 *
 * This file is the reading half and holds no JSX, so the rules can be tested
 * without a DOM. Getting the classification wrong is a cosmetic mistake and
 * never a lost result: whatever is not recognised still renders as itself, and
 * the raw JSON is always one click away.
 */

/* -- parsing -------------------------------------------------------------- */

/** The trailer `flattenContent` adds when a result was too big to send whole. */
const TRUNCATION = /\n?\[result truncated\]\s*$/;

/**
 * The JSON inside a tool's text, or `undefined` when there is none.
 *
 * Three shapes, because servers send all three: one document, one document
 * with the truncation trailer stuck to it, and a run of newline-separated
 * documents (NDJSON, which several database and log servers answer with). A
 * result that is prose stays prose - `undefined` means "draw this as text",
 * not "something went wrong".
 */
export function parseResult(output) {
  const text = String(output ?? "")
    .replace(TRUNCATION, "")
    .trim();
  if (!text) return undefined;
  // A bare number or `true` is JSON, and rendering it as a JSON document
  // rather than as the sentence it probably is helps nobody.
  if (!/^[[{]/.test(text)) return undefined;

  try {
    return JSON.parse(text);
  } catch {
    /* Not one document. It may still be several. */
  }

  const lines = text.split("\n").filter((line) => line.trim());
  if (lines.length < 2) return undefined;
  const rows = [];
  for (const line of lines) {
    try {
      rows.push(JSON.parse(line));
    } catch {
      return undefined;
    }
  }
  return rows;
}

/* -- shapes --------------------------------------------------------------- */

const isPlainObject = (value) =>
  Boolean(value) && typeof value === "object" && !Array.isArray(value);

/** A string long enough that it is a paragraph rather than a field. */
const TEXT_CHARS = 140;

/**
 * What this value is, in the only terms the renderer cares about.
 *
 * `table` is the one worth explaining: a list of objects that mostly agree
 * about their keys is a table, and drawing it as one is the difference between
 * eight rows a reader can compare and eight paragraphs they have to hold in
 * their head. A list of objects that do not agree is just a list.
 */
export function classify(value) {
  if (value == null) return "empty";
  if (typeof value === "string") {
    if (!value.trim()) return "empty";
    if (isImageUrl(value)) return "image";
    if (value.length > TEXT_CHARS || value.includes("\n")) return "text";
    return "scalar";
  }
  if (typeof value === "number" || typeof value === "boolean") return "scalar";
  if (Array.isArray(value)) {
    if (!value.length) return "empty";
    if (value.every((item) => item == null || typeof item !== "object")) return "list";
    if (value.every(isPlainObject) && columnsOf(value).length) return "table";
    return "list";
  }
  if (isPlainObject(value)) {
    return Object.keys(value).length ? "record" : "empty";
  }
  return "scalar";
}

/* -- fields --------------------------------------------------------------- */

/**
 * Field roles, by what the key is called.
 *
 * Guessing from names is not something to be proud of, but it is what is
 * available: MCP publishes a schema for a tool's arguments and nothing at all
 * for its results. The patterns are anchored so `url` and `image_url` match
 * while `curl_command` and `thumbnail_policy` do not, and a wrong guess costs
 * a field drawn as a link instead of as text.
 */
const ROLES = [
  [
    "image",
    /^(image|img|thumbnail|thumb|photo|picture|avatar|icon|screenshot|cover|poster)(_?(url|uri|src|link))?$/i,
  ],
  ["url", /^(url|uri|href|link|permalink|web_?url|html_?url|source_?url|download_?url)$/i],
  ["title", /^(title|name|heading|headline|subject|label|display_?name|full_?name)$/i],
  [
    "body",
    /^(content|text|description|snippet|summary|body|answer|message|excerpt|abstract|caption)$/i,
  ],
  ["score", /^(score|rating|relevance|confidence|similarity|rank|weight)$/i],
  [
    "time",
    /^(date|time|timestamp|published(_?(at|date|time))?|created(_?at)?|updated(_?at)?|modified(_?at)?)$/i,
  ],
  ["status", /^(status|state|error|errors|level|severity|result|outcome)$/i],
  ["id", /^(id|_?id|uuid|guid|key|slug|hash|sha|ref)$/i],
];

/** What a key means, or `"field"` when it means nothing in particular. */
export function roleOf(key, value) {
  const name = String(key ?? "");
  for (const [role, pattern] of ROLES) {
    if (!pattern.test(name)) continue;
    // A key called `image` holding an object is not a picture, and a `url`
    // holding a number is not a link. The name proposes; the value decides.
    if (role === "image") return isImageUrl(value) || isUrl(value) ? "image" : "field";
    if (role === "url") return isUrl(value) ? "url" : "field";
    if (role !== "image" && role !== "url" && typeof value === "object" && value !== null) {
      return "field";
    }
    return role;
  }
  return "field";
}

const URL_PATTERN = /^(https?:\/\/|data:|file:\/\/)\S+$/i;
const IMAGE_EXTENSION = /\.(png|jpe?g|gif|webp|avif|bmp|svg)(\?|#|$)/i;

/** Something that can be opened. */
export function isUrl(value) {
  return typeof value === "string" && URL_PATTERN.test(value.trim());
}

/**
 * Something that can be drawn.
 *
 * A `data:image/...` URI is certain. An http one is a guess from its
 * extension, and deliberately a narrow one: an `<img>` pointed at a page of
 * HTML draws a broken-image icon, which is worse than the link it replaced.
 */
export function isImageUrl(value) {
  if (typeof value !== "string") return false;
  const text = value.trim();
  if (/^data:image\//i.test(text)) return true;
  if (!URL_PATTERN.test(text)) return false;
  return IMAGE_EXTENSION.test(text);
}

/** A base64 attachment, as a URL an `<img>` can take. */
export function attachmentUrl(attachment) {
  if (!attachment) return "";
  if (isUrl(attachment.data)) return attachment.data;
  if (!attachment.data) return "";
  return `data:${attachment.mimeType || "application/octet-stream"};base64,${attachment.data}`;
}

/* -- tables --------------------------------------------------------------- */

/** Enough rows to be a shape rather than a coincidence. */
const COLUMN_SHARE = 0.6;
const MAX_COLUMNS = 6;

/**
 * The columns a list of objects actually has in common.
 *
 * Only scalars: a column holding an object is a column of "[object]", and the
 * row is better off keeping that field for its expanded body. Order follows
 * first appearance, because the server chose an order and it is usually the
 * useful one.
 */
export function columnsOf(rows, { max = MAX_COLUMNS } = {}) {
  const objects = (rows ?? []).filter(isPlainObject);
  if (objects.length < 2) return [];
  const counts = new Map();
  for (const row of objects) {
    for (const [key, value] of Object.entries(row)) {
      if (value !== null && typeof value === "object") continue;
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
  }
  const needed = Math.ceil(objects.length * COLUMN_SHARE);
  return [...counts.entries()]
    .filter(([, count]) => count >= needed)
    .slice(0, max)
    .map(([key]) => key);
}

/**
 * A table is for comparing, and comparing needs values short enough to sit
 * side by side. A list whose rows carry paragraphs is a list of cards however
 * uniform its keys are.
 */
const CELL_CHARS = 60;

export function fitsATable(rows) {
  const columns = columnsOf(rows);
  if (columns.length < 2) return false;
  for (const row of rows) {
    for (const key of columns) {
      const value = row?.[key];
      if (typeof value === "string" && value.length > CELL_CHARS) return false;
      if (roleOf(key, value) === "image") return false;
    }
  }
  // A row with much more in it than its columns show is a card with a table
  // squeezed out of it; the fields left over are the ones worth reading.
  return rows.every((row) => Object.keys(row ?? {}).length <= columns.length + 2);
}

/* -- cards ---------------------------------------------------------------- */

/**
 * One object, sorted into the parts a card draws.
 *
 * The title, the link, the picture and the paragraph are the four things a
 * result almost always has and a reader almost always wants first. Everything
 * else keeps its key and goes below, in the order the server sent it, so
 * nothing is ever dropped for not being recognised.
 */
export function cardOf(row) {
  const card = { title: "", url: "", image: "", body: "", fields: [], nested: [] };
  if (!isPlainObject(row)) return card;

  for (const [key, value] of Object.entries(row)) {
    if (value == null || value === "" || (Array.isArray(value) && !value.length)) continue;
    const role = roleOf(key, value);

    if (role === "title" && !card.title && typeof value === "string") {
      card.title = value;
      continue;
    }
    if (role === "url" && !card.url) {
      card.url = value;
      continue;
    }
    if (role === "image" && !card.image) {
      card.image = value;
      continue;
    }
    if (role === "body" && !card.body && typeof value === "string") {
      card.body = value;
      continue;
    }
    if (value !== null && typeof value === "object") {
      card.nested.push([key, value]);
      continue;
    }
    card.fields.push([key, value, role]);
  }

  return card;
}

/* -- words ---------------------------------------------------------------- */

/** `follow_up_questions` → `Follow up questions`. A key, said out loud. */
export function labelOf(key) {
  const words = String(key ?? "")
    .replace(/[_-]+/g, " ")
    .replace(/([a-z\d])([A-Z])/g, "$1 $2")
    .trim()
    .toLowerCase();
  if (!words) return "";
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** What is behind a fold, in a few words, so it can be skipped unopened. */
export function preview(value) {
  const shape = classify(value);
  if (shape === "empty") return "empty";
  if (Array.isArray(value)) return value.length === 1 ? "1 item" : `${value.length} items`;
  if (isPlainObject(value)) {
    const keys = Object.keys(value);
    return keys.length <= 3 ? keys.join(", ") : `${keys.length} fields`;
  }
  const text = String(value).replace(/\s+/g, " ").trim();
  return text.length > 80 ? `${text.slice(0, 80)}…` : text;
}

/**
 * The colour a scalar earns, out of the five the theme has.
 *
 * By meaning and never at random: a red chip has to mean something went wrong
 * every time it appears, or it means nothing anywhere. Anything unrecognised
 * stays neutral, which is most things.
 */
const BAD = /^(error|failed|failure|denied|invalid|unhealthy|down|critical|fatal|rejected)$/i;
const GOOD =
  /^(ok|success|succeeded|passed|healthy|active|connected|complete|completed|done|up|valid)$/i;
const WARN = /^(warn|warning|pending|degraded|partial|stale|retrying|queued|skipped)$/i;

export function toneOf(key, value) {
  if (typeof value === "boolean") return value ? "success" : "neutral";
  if (roleOf(key, value) === "score") return "info";
  const text = String(value ?? "").trim();
  if (BAD.test(text)) return "danger";
  if (GOOD.test(text)) return "success";
  if (WARN.test(text)) return "warning";
  if (/^(error|errors)$/i.test(String(key ?? "")) && text) return "danger";
  return "neutral";
}

/**
 * A record's headline field, when it has one worth lifting out of the list.
 *
 * Search servers answer with the answer and then the sources; a card that
 * opens on `answer` and folds the rest is the whole result in one line, and
 * the same rule serves anything that has one long field among short ones.
 */
export function leadOf(record) {
  if (!isPlainObject(record)) return "";
  for (const [key, value] of Object.entries(record)) {
    if (typeof value !== "string" || value.length <= TEXT_CHARS) continue;
    if (roleOf(key, value) === "body") return key;
  }
  return "";
}
