import { describe, it, expect } from "vitest";
import { collectThreadFiles } from "@/features/chat/thread-files";
import {
  collectLibraryArtifacts,
  dataUrlPayload,
  libraryCategory,
  mimeFor,
  openTargetFor,
  safeFileName,
  saveTargetPath,
  toLibraryItem,
} from "@/features/library/artifacts";

/**
 * How an artifact row is derived from a message.
 *
 * The whole Library is now a view over the transcripts, so every mistake here
 * is a file that either does not appear on the screen or appears claiming to be
 * something it is not. The two failures worth pinning down are the ones that
 * are silent: an attachment whose bytes are dropped on the way out of the
 * message looks like a path artifact and offers the wrong buttons, and a path
 * routed to the workspace channel instead of the OS one is refused for
 * escaping a folder it was never in.
 */

const imageAttachment = {
  id: "att-1",
  name: "screenshot.png",
  kind: "image",
  size: 2048,
  dataUrl: "data:image/png;base64,AAAA",
};

const textAttachment = {
  id: "att-2",
  name: "notes.md",
  kind: "text",
  size: 12,
  text: "# Notes\nhello",
};

const thread = { id: "t1", title: "Ship the thing", agentId: "agent-1" };

const blocks = [
  {
    id: "m1",
    role: "user",
    createdAt: "2026-09-01T10:00:00.000Z",
    attachments: [imageAttachment, textAttachment],
  },
  {
    id: "m2",
    type: "tool",
    name: "write",
    agentId: "agent-2",
    createdAt: "2026-09-01T11:00:00.000Z",
    args: { filePath: "/home/dev/app/README.md" },
  },
];

/** A live reply, whose tool calls are parts rather than blocks of their own. */
const replyWithParts = {
  id: "m4",
  role: "assistant",
  agentId: "agent-2",
  createdAt: "2026-09-01T13:00:00.000Z",
  parts: [
    { type: "text", text: "Writing it now." },
    { type: "tool", callId: "c1", name: "edit", args: { filePath: "/home/dev/app/index.js" } },
  ],
};

describe("collectThreadFiles, with real attachments", () => {
  it("carries the bytes of an attachment out with the row", () => {
    const [image, text] = collectThreadFiles([blocks[0]]);
    expect(image.dataUrl).toBe(imageAttachment.dataUrl);
    expect(image.sizeBytes).toBe(2048);
    expect(text.text).toBe("# Notes\nhello");
  });

  it("gives a text attachment the glyph its extension earns, not `text`", () => {
    const [, text] = collectThreadFiles([blocks[0]]);
    expect(text.kind).toBe("markdown");
  });

  it("keeps two attachments that happen to share a name", () => {
    const twice = [
      { ...blocks[0], attachments: [imageAttachment] },
      {
        ...blocks[0],
        id: "m3",
        createdAt: "2026-09-01T12:00:00.000Z",
        attachments: [{ ...imageAttachment, id: "att-9" }],
      },
    ];
    expect(collectThreadFiles(twice)).toHaveLength(2);
  });

  it("leaves a tool path without bytes to show", () => {
    const [written] = collectThreadFiles([blocks[1]]);
    expect(written.origin).toBe("written");
    expect(written.name).toBe("README.md");
    expect(written.dataUrl).toBeUndefined();
    expect(written.text).toBeUndefined();
  });

  it("detects a file written by a tool part inside a live reply", () => {
    // This is the shape a running turn produces, and the one the panel was
    // blind to: the write is a part of the reply, not a block of its own.
    const [written] = collectThreadFiles([replyWithParts]);
    expect(written.origin).toBe("written");
    expect(written.path).toBe("/home/dev/app/index.js");
    expect(written.agentId).toBe("agent-2");
  });

  it("shows one row per file even when it is written more than once", () => {
    const twice = [blocks[1], { ...blocks[1], id: "m5", createdAt: "2026-09-01T14:00:00.000Z" }];
    const rows = collectThreadFiles(twice);
    expect(rows).toHaveLength(1);
    expect(rows[0].at).toBe("2026-09-01T14:00:00.000Z");
  });
});

describe("libraryCategory", () => {
  it("puts an image under images and a spreadsheet under data", () => {
    expect(libraryCategory("shot.png", "image")).toBe("image");
    expect(libraryCategory("rows.csv", "table")).toBe("data");
  });

  it("recognises media by extension, since no attachment can be one", () => {
    expect(libraryCategory("take.mp3", "file")).toBe("audio");
    expect(libraryCategory("demo.mp4", "file")).toBe("video");
  });

  it("files anything unrecognised under documents rather than dropping it", () => {
    expect(libraryCategory("LICENSE", "file")).toBe("document");
  });
});

describe("mimeFor", () => {
  it("believes the data URL over the filename", () => {
    expect(mimeFor({ name: "shot.png", dataUrl: "data:image/webp;base64,AA" })).toBe("image/webp");
  });

  it("falls back to the extension, then to bytes of unknown type", () => {
    expect(mimeFor({ name: "notes.md" })).toBe("text/markdown");
    expect(mimeFor({ name: "thing.bin" })).toBe("application/octet-stream");
  });
});

describe("dataUrlPayload", () => {
  it("returns just the base64 half", () => {
    expect(dataUrlPayload("data:image/png;base64,QUJD")).toBe("QUJD");
  });

  it("refuses anything that is not a data URL", () => {
    expect(dataUrlPayload("/tmp/shot.png")).toBeNull();
    expect(dataUrlPayload(null)).toBeNull();
  });
});

describe("safeFileName and saveTargetPath", () => {
  it("strips what would let a name write outside the folder it was aimed at", () => {
    expect(safeFileName("../../etc/passwd")).toBe("-.-etc-passwd");
    expect(safeFileName(".ssh")).toBe("ssh");
    expect(safeFileName("")).toBe("artifact");
  });

  it("scopes a save under its thread, so two reports do not collide", () => {
    expect(saveTargetPath({ threadId: "t1", name: "report.md" })).toBe("files/t1/report.md");
    expect(saveTargetPath({ name: "report.md" })).toBe("files/chat/report.md");
  });
});

describe("openTargetFor", () => {
  it("sends an absolute path to the OS, on either platform's spelling", () => {
    expect(openTargetFor("/home/dev/app/README.md").how).toBe("os");
    expect(openTargetFor("C:\\Users\\dev\\notes.txt").how).toBe("os");
    expect(openTargetFor("\\\\server\\share\\file.txt").how).toBe("os");
  });

  it("sends a relative path to the workspace, which is the only base it knows", () => {
    expect(openTargetFor("files/t1/report.md").how).toBe("workspace");
  });

  it("has nothing to open when there is no path", () => {
    expect(openTargetFor("").how).toBe("unknown");
  });
});

describe("toLibraryItem", () => {
  it("borrows the thread's agent for a file the user attached themselves", () => {
    const [image] = collectThreadFiles([blocks[0]]);
    expect(image.agentId).toBeNull();
    expect(toLibraryItem(image, thread).agentId).toBe("agent-1");
  });

  it("keeps the agent that wrote a file, over the thread's", () => {
    const [written] = collectThreadFiles([blocks[1]]);
    expect(toLibraryItem(written, thread).agentId).toBe("agent-2");
  });

  it("names the origin the detail sheet reads", () => {
    const [image] = collectThreadFiles([blocks[0]]);
    const [written] = collectThreadFiles([blocks[1]]);
    expect(toLibraryItem(image, thread).source).toBe("attachment");
    expect(toLibraryItem(written, thread).source).toBe("tool");
  });
});

describe("collectLibraryArtifacts", () => {
  const catalogue = collectLibraryArtifacts({ threads: [thread], messages: { t1: blocks } });

  it("finds everything a thread produced, of both kinds", () => {
    expect(catalogue.map((item) => item.name)).toEqual(["README.md", "screenshot.png", "notes.md"]);
  });

  it("scopes ids by thread, so two transcripts cannot collide", () => {
    expect(catalogue.every((item) => item.id.startsWith("t1:"))).toBe(true);
  });

  it("is empty when nothing has been said", () => {
    expect(collectLibraryArtifacts({ threads: [thread], messages: {} })).toEqual([]);
    expect(collectLibraryArtifacts()).toEqual([]);
  });
});
