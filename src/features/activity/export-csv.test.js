import { describe, it, expect } from "vitest";
import { escapeCsvField, exportFileName, toCsv } from "./export-csv.js";

/**
 * An export is read by a spreadsheet, and a spreadsheet does not report a
 * malformed file - it silently puts half a sentence in the next column. So the
 * cases here are the ones that shift columns: a comma in a title, a quote in a
 * detail, a newline in an error message an agent wrote.
 */

describe("escaping a field", () => {
  it("leaves a plain field alone, so the file stays readable", () => {
    expect(escapeCsvField("Ran the nightly brief")).toBe("Ran the nightly brief");
  });

  it("quotes a field holding a comma", () => {
    expect(escapeCsvField("Read, then wrote")).toBe('"Read, then wrote"');
  });

  it("doubles a quote inside a quoted field", () => {
    expect(escapeCsvField('He said "no"')).toBe('"He said ""no"""');
  });

  it("quotes a field holding a newline rather than ending the record", () => {
    expect(escapeCsvField("line one\nline two")).toBe('"line one\nline two"');
    expect(escapeCsvField("line one\r\nline two")).toBe('"line one\r\nline two"');
  });

  it("writes an empty field for a missing value, not the word null", () => {
    expect(escapeCsvField(null)).toBe("");
    expect(escapeCsvField(undefined)).toBe("");
  });

  it("writes a number as its digits", () => {
    expect(escapeCsvField(0)).toBe("0");
  });
});

describe("the document", () => {
  const event = {
    at: "2026-09-01T08:00:00.000Z",
    severity: "warning",
    category: "tool",
    agentId: "agent-1",
    actor: "agent",
    title: "Blocked a command",
    detail: 'rm -rf /, which the rules call "never"',
    target: "shell",
  };

  it("names the agent rather than its id", () => {
    const csv = toCsv([event], [{ id: "agent-1", name: "Atlas" }]);
    expect(csv.split("\r\n")[1]).toContain("Atlas");
  });

  it("keeps a value carrying a comma and a quote in one field", () => {
    const [, row] = toCsv([event], []).split("\r\n");
    // Eight columns, and the awkward detail is one of them rather than three.
    expect(row).toContain('"rm -rf /, which the rules call ""never"""');
  });

  it("ends every record with CRLF, which is what the format says", () => {
    expect(toCsv([event], [])).toMatch(/\r\n$/);
  });

  it("writes the header even when there is nothing to export", () => {
    expect(toCsv([], [])).toBe(
      "at,severity,category,agent,actor,title,detail,target\r\n"
    );
  });

  it("falls back to the id when the agent is gone", () => {
    expect(toCsv([event], [])).toContain("agent-1");
  });
});

describe("the file name", () => {
  it("carries the minute it was taken, so a second export is a second file", () => {
    expect(exportFileName(new Date(Date.UTC(2026, 8, 1, 8, 30)))).toBe(
      "activity-log-2026-09-01-08-30.csv"
    );
  });
});
