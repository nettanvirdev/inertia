/**
 * The activity feed, as a file someone can open in a spreadsheet.
 *
 * Pure on purpose. Turning rows into CSV is the half of an export that can be
 * wrong in ways nobody notices until a colleague opens the file and finds the
 * columns shifted by one, so it is a function over records with no workspace,
 * no toast and no browser in it, and it is tested.
 *
 * The button this belongs to used to raise "N events queued as
 * activity-log.csv" and write nothing at all, which is the failure this file
 * exists to end.
 */

/**
 * One field, quoted the way RFC 4180 says.
 *
 * Everything that could end a field or a record has to be inside quotes, and a
 * quote inside a quoted field is written twice. Agent titles carry commas and a
 * tool's error message carries newlines, so this is the common case rather than
 * the careful one - a field that needs no quoting is written bare so the file
 * stays readable in a text editor.
 *
 * Null and undefined are an empty field rather than the word "null": a missing
 * detail is missing, not the string.
 */
export function escapeCsvField(value) {
  if (value == null) return "";
  const text = typeof value === "string" ? value : String(value);
  if (!/[",\n\r]/.test(text)) return text;
  return `"${text.replace(/"/g, '""')}"`;
}

/** The columns, in the order they are written. */
export const CSV_COLUMNS = [
  ["at", (event) => event.at],
  ["severity", (event) => event.severity ?? "info"],
  ["category", (event) => event.category],
  ["agent", (event, names) => names.get(event.agentId) ?? event.agentId],
  ["actor", (event) => event.actor],
  ["title", (event) => event.title],
  ["detail", (event) => event.detail],
  ["target", (event) => event.target],
];

/**
 * The events as a CSV document.
 *
 * Records end with CRLF, which is what the format says and what every
 * spreadsheet on Windows expects. The header is always written, so an export of
 * nothing is still a file that opens and says what it would have held.
 *
 * `agents` is passed so the file names the teammate rather than its id. The id
 * is what survives in the folder, but a spreadsheet is read by people.
 */
export function toCsv(events = [], agents = []) {
  const names = new Map((agents ?? []).map((agent) => [agent.id, agent.name]));
  const lines = [CSV_COLUMNS.map(([name]) => name).join(",")];
  for (const event of events) {
    lines.push(CSV_COLUMNS.map(([, read]) => escapeCsvField(read(event, names))).join(","));
  }
  return lines.join("\r\n") + "\r\n";
}

/**
 * Where one export lands.
 *
 * Stamped with the minute it was taken, because exporting twice in an afternoon
 * is normal and the second one silently replacing the first is not something a
 * person would find out about until they needed the first.
 */
export function exportFileName(now = new Date()) {
  const stamp = now.toISOString().slice(0, 16).replace(/[:T]/g, "-");
  return `activity-log-${stamp}.csv`;
}
