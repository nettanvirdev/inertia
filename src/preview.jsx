import { StrictMode, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles/globals.css";
import { ToastProvider } from "./components/ui/toast.jsx";
import { WorkspaceProvider } from "./lib/workspace.jsx";
import { Markdown } from "./features/chat/Markdown.jsx";
import { ToolCallCard, ToolRun } from "./features/chat/ToolCallCard.jsx";
import { PresentedFiles } from "./features/chat/PresentedFiles.jsx";
import { ChangesStrip } from "./features/chat/ChangesStrip.jsx";
import { MessageParts } from "./features/chat/MessageParts.jsx";
import { SideDock } from "./features/chat/SideDock.jsx";
import { PromptEditor } from "./features/chat/PromptEditor.jsx";
import { MentionPalette, useMentionPalette } from "./features/chat/MentionPalette.jsx";
import { mentionSpecs } from "./features/chat/mentions.js";
import { renderToStaticMarkup } from "react-dom/server";
import { Icon } from "./components/icons/index.jsx";
import { groupParts } from "./features/chat/tool-groups.js";
import { IconButton } from "./components/ui/icon-button.jsx";
import { Copy } from "./components/icons/index.jsx";
import { MemoryCard } from "./features/memory/MemoryCard.jsx";

/**
 * A page for looking at message rendering, outside the app.
 *
 * The transcript is the one screen whose contents come from a model rather
 * than from the app, so the only way to see a table beside a diagram beside a
 * block of Rust is to have written the message that has all three. This is
 * that message, served at /preview.html by the dev server. It is not reachable
 * from the app, is not in the packaged build, and exists so a change to the
 * markdown renderer can be looked at rather than asserted about.
 */

// A raw string, so every backslash in the maths survives. Backticks cannot be
// written inside one - an escape keeps its backslash in a raw string - so the
// fences and the inline code come in through these.
const T = "```";
const B = "`";

const PICTURE =
  "data:image/png;base64,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";

const ASK = String.raw`Build a **dentist clinic website**.

## Rules

* White mode only, no dark mode
* No gradients, no drop shadows
* Keep it minimal

Start from ${B}app/page.tsx${B} and tell me when the hero is done.`;

const SAMPLE = String.raw`# Rendering, all of it

A paragraph with **bold**, *italic*, ~~struck~~ text, ${B}inline code${B}, and a
[link](https://example.com). Here is a footnote-ish aside in a quote:

> The folder wins. Anything the app shows is a file, and the file is the truth.

## Lists

- One, with a nested list
  - Two
  - Three, which is long enough to wrap when the message column is narrow, so the
    indent has somewhere to show itself
- [x] A finished task
- [ ] An unfinished one

1. First
2. Second
3. Third

## A table

| Tool | Asks under | Danger |
| --- | :---: | ---: |
| ${B}read${B} | read | low |
| ${B}file_move${B} | edit | high |
| ${B}file_delete${B} | edit, then delete_everything | high |

## Code

${T}js
// A comment about the thing below.
import { readFile } from "node:fs/promises";

export async function load(path, { limit = 2000 } = {}) {
  const text = await readFile(path, "utf8");
  return text.split("
").slice(0, limit).map((line, i) => i + 1 + ": " + line);
}
${T}

${T}python
@cache
def fibonacci(n: int) -> int:
    """The slow one, on purpose."""
    if n < 2:
        return n
    return fibonacci(n - 1) + fibonacci(n - 2)  # 0.5s at n=35
${T}

${T}diff
--- a/src/main/tools/registry.cjs
+++ b/src/main/tools/registry.cjs
@@ -30,6 +30,7 @@
   require("./builtin/patch.cjs"),
-  require("./builtin/shell.cjs"),
+  ...require("./builtin/files.cjs").tools,
${T}

${T}json
{ "name": "inertia", "version": "0.1.0", "private": true, "count": 42 }
${T}

## Maths

Inline: the cost of a turn is $c = n \cdot p_{in} + m \cdot p_{out}$, which is
linear in both.

$$
\frac{\partial L}{\partial w_i} = \sum_{j=1}^{n} \left( \hat{y}_j - y_j \right) x_{ij}
$$

$$
P(A \mid B) = \frac{P(B \mid A)\,P(A)}{P(B)} \qquad \alpha + \beta \geq \sqrt{\gamma}
$$

## A flowchart

${T}mermaid
flowchart TD
  A[User asks] --> B{Mode?}
  B -->|chat| C([Answer])
  B -->|autonomous| D[Run tools]
  D --> E{Permission}
  E -- allowed --> F[(Write files)]
  E -- refused --> G([Stop the turn])
  F --> H[Verify]
  H --> D
${T}

## A sequence

${T}mermaid
sequenceDiagram
  participant U as User
  participant A as Agent
  participant P as Provider
  U->>A: fix the login bug
  A->>P: prompt + tools
  P-->>A: tool call
  A->>A: run it
  Note right of A: permission card
  A-->>U: a diff and a sentence
${T}

## Rendered markup

${T}html
<div style="font-family: system-ui; padding: 16px">
  <h3 style="margin: 0 0 8px">A card</h3>
  <p style="margin: 0; color: #555">Rendered in a sandboxed frame, with no scripts.</p>
</div>
${T}

A [link to somewhere](https://example.com/docs/getting-started?x=1) to hover.

## Markup a model wrote inline

Here is a picture written as a tag: <img src="${PICTURE}" alt="Inline tag" width="220"/>

Text with <b>bold</b>, <i>italic</i>, <kbd>Ctrl</kbd>+<kbd>C</kbd>, <mark>a mark</mark>,
H<sub>2</sub>O and E=mc<sup>2</sup>, and a claim with a note[^1].

[^1]: This is a footnote example, at the foot of the message where it belongs.

[^2]: A second note, numbered after the first.

## A runnable command

${T}bash
git status --short
${T}

## A runnable program

${T}python
def fibonacci(n: int) -> int:
    """Return the n-th Fibonacci number."""
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a

print(fibonacci(10))  # 55
${T}

## A picture

![a picture](${PICTURE})

Text after it, so the spacing around a picture can be judged.

## A subgraph

${T}mermaid
flowchart LR
  subgraph Renderer
    ui[Chat view] --> store[Workspace store]
  end
  subgraph Main
    ipc[IPC] --> fs[(Workspace folder)]
  end
  store --> ipc
${T}
`;

// The preview page has no backend behind it. A stand-in for the two calls
// a message can make lets the Run button and a remote picture be looked at
// here rather than only in the packaged app.
if (!window.electronAPI) {
  window.electronAPI = {
    runSnippet: async ({ command, cwd }) =>
      new Promise((resolve) =>
        setTimeout(
          () =>
            resolve({
              ok: true,
              code: 0,
              cwd: cwd ?? "C:/Users/you/Documents/Inertia",
              command,
              output: [
                " M src/renderer/features/chat/markdown/CodeBlock.jsx",
                "?? src/renderer/preview.jsx",
              ].join("\n"),
            }),
          600
        )
      ),
    fetchImage: async () => null,
    saveImage: async ({ name }) => ({ saved: true, path: `C:/Users/you/Downloads/${name}.png` }),
    canRun: async (lang) =>
      ["python", "js", "javascript", "ts"].includes(String(lang).toLowerCase()),
  };
}

/** Agents and skills to mention, standing in for a real workspace. */
const CAST = [
  { id: "a1", name: "Nova", role: "Engineer", icon: "Settings", avatarColor: "#6366f1" },
  { id: "a2", name: "Iris", role: "Designer", initials: "IR", avatarColor: "#ec4899" },
  { id: "a3", name: "Wren", role: "Researcher", initials: "WR", avatarColor: "#14b8a6" },
  { id: "a4", name: "Nova Reyes", role: "Reviewer", initials: "NR", avatarColor: "#f59e0b" },
];
const SKILLS = [
  { id: "s1", name: "Brand Guidelines" },
  { id: "s2", name: "Code Review" },
  { id: "s3", name: "Frontend Design" },
];

/** The composer's text surface on its own, so the pills can be looked at. */
function ComposerDemo() {
  const [value, setValue] = useState("@nova and @iris, please run /code-review on the router.");
  const editorRef = useRef(null);
  const nodeRef = useRef(null);
  const mentions = useMemo(
    () =>
      mentionSpecs({
        agents: CAST,
        skills: SKILLS,
        face: (agent) => ({
          iconHtml: agent.icon ? renderToStaticMarkup(<Icon name={agent.icon} />) : null,
        }),
      }),
    []
  );
  const palette = useMentionPalette({ mentions, editorRef, nodeRef, onChange: setValue });

  return (
    <div>
      <div className="rounded-3xl bg-input p-1">
        <PromptEditor
          ref={editorRef}
          elementRef={nodeRef}
          value={value}
          onChange={setValue}
          mentions={mentions}
          onCaretChange={palette.track}
          onKeyDown={(e) => {
            if (palette.onKeyDown(e)) e.preventDefault();
          }}
          aria-label="Message"
          placeholder="Send a message…"
          className="px-4 py-3 text-base leading-relaxed"
        />
      </div>
      <MentionPalette palette={palette} agents={CAST} />
      <pre className="mt-2 whitespace-pre-wrap font-mono text-[11px] text-muted-foreground">
        {value}
      </pre>
    </div>
  );
}

function Page() {
  const [dark, setDark] = useState(true);
  // In an effect, not during render: React runs a render twice under
  // StrictMode, and the second one was putting the class back.
  useEffect(() => {
    document.documentElement.className = dark ? "dark" : "light";
  }, [dark]);
  return (
    // The app's own body never scrolls - the shell owns that - so this page
    // brings its own scroller rather than editing a rule the real window
    // depends on.
    <div className="h-full overflow-y-auto bg-background p-8">
      <button
        type="button"
        onClick={() => setDark((value) => !value)}
        className="mb-6 rounded-lg fill-control px-3 py-1.5 text-xs text-foreground"
      >
        {dark ? "Light" : "Dark"}
      </button>
      <div className="mx-auto max-w-3xl">
        {/* The memory calls, which are the one family that reports itself only
            while it is happening. Running shows a line; done shows nothing at
            all; failed stays, because a memory that silently did not save is
            the bug this would otherwise hide. */}
        <div className="mb-8 rounded-2xl fill-whisper p-4">
          <p className="mb-2 text-xs text-muted-foreground">Quiet calls</p>
          <ToolCallCard
            part={{ callId: "1", name: "memory_recall", title: "Recall", state: "running" }}
          />
          <ToolCallCard
            part={{ callId: "2", name: "memory_save", title: "Remember", state: "running" }}
          />
          <ToolCallCard
            part={{ callId: "3", name: "memory_forget", title: "Forget", state: "running" }}
          />
          <p className="mb-2 mt-4 text-xs text-muted-foreground">Finished (renders nothing)</p>
          <ToolCallCard
            part={{
              callId: "4",
              name: "memory_save",
              title: "Remember",
              state: "done",
              output: "Saved as mem-1.",
            }}
          />
          <p className="mb-2 mt-4 text-xs text-muted-foreground">Failed (stays)</p>
          <ToolCallCard
            part={{
              callId: "5",
              name: "memory_save",
              title: "Remember",
              state: "failed",
              output: "That looks like it contains a key, so it was not saved.",
            }}
          />
          <p className="mb-2 mt-4 text-xs text-muted-foreground">
            An ordinary call, for comparison
          </p>
          <ToolCallCard
            part={{
              callId: "6",
              name: "read",
              title: "src/main/memory/index.cjs",
              state: "done",
              output: "...",
            }}
          />
        </div>
        {/* Memories, in the three states that are easy to get wrong: one that
            applies everywhere, one that belongs to a project, and one that has
            been written down but is not believed by anything yet. */}
        <div className="mb-8 grid gap-3 sm:grid-cols-2">
          <MemoryCard
            compact
            memory={{
              id: "1",
              title: "Pushes straight to master",
              body: "Inertia work goes to master, never a feature branch.",
              kind: "preference",
              source: "learned",
              scope: "global",
              pinned: true,
              confidence: 1,
              useCount: 12,
              lastUsedAt: new Date().toISOString(),
              tags: [],
            }}
            onPin={() => {}}
            onEdit={() => {}}
            onDelete={() => {}}
          />
          <MemoryCard
            compact
            memory={{
              id: "2",
              title: "Handlers live in src/routes",
              body: "Grouped by resource, one folder each.",
              kind: "fact",
              source: "learned",
              scope: "project",
              folder: "D:/work/api",
              confidence: 1,
              useCount: 3,
              lastUsedAt: new Date(Date.now() - 40 * 86400000).toISOString(),
              tags: [],
            }}
            onPin={() => {}}
            onEdit={() => {}}
            onDelete={() => {}}
          />
          <MemoryCard
            compact
            memory={{
              id: "3",
              title: "Prefers pnpm",
              body: "Said so while fixing the lockfile.",
              kind: "preference",
              source: "learned",
              scope: "project",
              folder: "D:/work/website",
              pending: true,
              confidence: 0.4,
              useCount: 0,
              tags: [],
            }}
            onPin={() => {}}
            onEdit={() => {}}
            onDelete={() => {}}
            onApprove={() => {}}
          />
          <MemoryCard
            compact
            memory={{
              id: "4",
              title: "Where we left off in api",
              body: "Routes were reorganised. The nested tests still fail.",
              kind: "handover",
              source: "learned",
              scope: "project",
              folder: "D:/work/api",
              pinned: true,
              confidence: 1,
              useCount: 1,
              lastUsedAt: new Date().toISOString(),
              tags: ["handover"],
            }}
            onPin={() => {}}
            onEdit={() => {}}
            onDelete={() => {}}
          />
        </div>
        <p className="mb-2 text-xs text-muted-foreground">The composer, with mentions</p>
        {/* The palette draws real agent avatars, and those are read through the
            workspace - so the demo needs one, even though nothing here writes
            to it. */}
        <div className="mb-8">
          <WorkspaceProvider>
            <ComposerDemo />
          </WorkspaceProvider>
        </div>

        {/* The dock: nothing, one section, or two with a divider between
            them. Rendered against a fixed height so the split is visible. */}
        <p className="mb-2 text-xs text-muted-foreground">The dock, with nothing open</p>
        <div
          className="mb-4 flex h-16 overflow-hidden rounded-2xl border border-border-subtle"
          data-closed-dock
        >
          <div className="min-w-0 flex-1 p-3 text-xs text-muted-foreground">
            the conversation, full width
          </div>
          <SideDock
            sections={[
              { id: "thread", open: false, node: <div>thread</div> },
              { id: "agents", open: false, node: <div>agents</div> },
            ]}
          />
        </div>

        <p className="mb-2 text-xs text-muted-foreground">The dock</p>
        <div className="mb-8 flex h-[22rem] overflow-hidden rounded-2xl border border-border-subtle">
          <div className="min-w-0 flex-1 p-3 text-xs text-muted-foreground">the conversation</div>
          <SideDock
            sections={[
              {
                id: "thread",
                open: true,
                node: (
                  <div className="h-full overflow-y-auto p-3">
                    <p className="mb-2 text-[12px] font-medium">Thread</p>
                    {Array.from({ length: 40 }, (_, i) => (
                      <p key={i} className="text-[11px] text-muted-foreground">
                        row {i + 1}
                      </p>
                    ))}
                  </div>
                ),
              },
              {
                id: "agents",
                open: true,
                node: (
                  <div className="h-full overflow-y-auto p-3">
                    <p className="mb-2 text-[12px] font-medium">Agents</p>
                    {Array.from({ length: 40 }, (_, i) => (
                      <p key={i} className="text-[11px] text-muted-foreground">
                        run {i + 1}
                      </p>
                    ))}
                  </div>
                ),
              },
            ]}
          />
        </div>

        {/* A helper's run, drawn with the transcript's own renderer at the
            size the Agents column uses. The brief sits where a user message
            would, because that is what it is. */}
        <p className="mb-2 text-xs text-muted-foreground">A helper, in the Agents column</p>
        <div className="mb-8 w-[22rem] rounded-2xl card-surface-subtle p-2">
          <div className="chat-scale">
            <div className="mb-1 flex justify-end">
              <div className="max-w-[85%] rounded-2xl rounded-br-md bg-user-message-background px-3 py-2 wrap-break-word">
                <Markdown>{`Build the **illustration system**. Steps:\n\n1. Read \`package.json\`\n2. Create eight files under \`components/illustrations/\`\n3. Report back`}</Markdown>
              </div>
            </div>
            <MessageParts
              streaming
              items={groupParts([
                {
                  type: "reasoning",
                  text: "Looking at the project first.",
                  startedAt: 1,
                  endedAt: 4200,
                },
                {
                  type: "tool",
                  callId: "t1",
                  name: "read",
                  title: "package.json",
                  state: "done",
                  durationMs: 2,
                  args: { filePath: "D:/x/package.json" },
                  output: "{}",
                },
                {
                  type: "tool",
                  callId: "t2",
                  name: "write",
                  title: "Tooth.tsx",
                  state: "done",
                  durationMs: 6,
                  metadata: { path: "D:/x/components/illustrations/Tooth.tsx", existed: false },
                  args: {
                    filePath: "D:/x/components/illustrations/Tooth.tsx",
                    content: `export function Tooth() {\n  return <svg viewBox="0 0 24 24" />;\n}\n`,
                  },
                  output: "Created Tooth.tsx with 3 lines",
                },
                {
                  type: "text",
                  text: `Created the first illustration. Here is what it covers:\n\n- \`Tooth\` for the hero\n- \`ToothAnatomy\` for the explainer\n\n| File | Lines |\n| --- | ---: |\n| Tooth.tsx | 226 |`,
                },
              ])}
            />
          </div>
        </div>

        {/* Folded by default: thirty file names under a reply are longer than
            the reply, and the summary is what the line is for. */}
        <p className="mb-2 text-xs text-muted-foreground">What a turn changed</p>
        <div className="mb-4">
          <ChangesStrip
            changes={{
              cwd: "C:/work/clinic-site",
              files: [
                { path: "app/globals.css", status: "M", additions: 212, deletions: 14 },
                { path: "app/layout.tsx", status: "M", additions: 41, deletions: 10 },
                { path: "components/sections/Hero.tsx", status: "A", additions: 454, deletions: 0 },
                {
                  path: "components/sections/Navbar.tsx",
                  status: "A",
                  additions: 337,
                  deletions: 0,
                },
              ],
            }}
          />
        </div>

        {/* An icon with no words says what it does on hover, everywhere. */}
        <p className="mb-2 text-xs text-muted-foreground">Hover me</p>
        <div className="mb-8">
          <IconButton size="md" label="Copy this thing">
            <Copy />
          </IconButton>
        </div>

        {/* What the agent hands over at the end. Any file type: the last one
            is deliberately an extension nothing here has heard of. */}
        <p className="mb-2 text-xs text-muted-foreground">Files handed over</p>
        <div className="mb-8">
          <PresentedFiles
            note="The site is built. These are the pages worth looking at."
            files={[
              { path: "C:/work/clinic-site/app/page.tsx", name: "page.tsx", bytes: 8134 },
              { path: "C:/work/clinic-site/app/globals.css", name: "globals.css", bytes: 3725 },
              { path: "C:/work/clinic-site/public/hero.png", name: "hero.png", bytes: 184320 },
              { path: "C:/work/clinic-site/out/build.qzx", name: "build.qzx", bytes: 91 },
            ]}
          />
        </div>

        {/* A file the agent created. There is no diff for a file that did not
            exist, which is exactly the case that used to render as a truncated
            line of arguments. */}
        <p className="mb-2 text-xs text-muted-foreground">A new file, written</p>
        <div className="mb-8">
          <ToolCallCard
            part={{
              callId: "w1",
              name: "write",
              title: "Doctors.tsx",
              state: "done",
              durationMs: 2,
              metadata: { path: "C:/work/clinic-site/app/components/Doctors.tsx", existed: false },
              args: {
                filePath: "C:/work/clinic-site/app/components/Doctors.tsx",
                content: `import Image from "next/image";
import { ArrowRightIcon } from "./Icons";

const doctors = [
  {
    name: "Dr. Elena Marquez",
    specialty: "Cosmetic & Restorative Dentistry",
    bio: "With 14 years of practice.",
  },
];

export default function Doctors() {
  return (
    <section className="mx-auto max-w-6xl px-6 py-20">
      <h2 className="text-3xl">Meet the team</h2>
    </section>
  );
}
`,
              },
              output: "Created app/components/Doctors.tsx with 97 lines",
            }}
          />
        </div>

        {/* A run of reads. The thing this replaces was eight identical rows
            that pushed the answer off the screen. */}
        <p className="mb-2 text-xs text-muted-foreground">A run of reads, folded</p>
        <div className="mb-8">
          <ToolRun
            calls={[
              {
                callId: "r1",
                name: "read",
                title: "package.json",
                state: "done",
                durationMs: 2,
                metadata: { path: "D:/work/site/package.json", lines: 41 },
                args: { filePath: "D:/work/site/package.json" },
                output: '{\n  "name": "site"\n}',
              },
              {
                callId: "r2",
                name: "read",
                title: "AGENTS.md",
                state: "done",
                durationMs: 2,
                metadata: { path: "D:/work/site/AGENTS.md", lines: 12 },
                args: { filePath: "D:/work/site/AGENTS.md" },
                output: "# Agents\n\nRun the tests.",
              },
              {
                callId: "r3",
                name: "read",
                title: "layout.tsx",
                state: "done",
                durationMs: 7,
                metadata: { path: "D:/work/site/app/layout.tsx", lines: 60 },
                args: { filePath: "D:/work/site/app/layout.tsx" },
                output: "export default function Layout() {}",
              },
              {
                callId: "r4",
                name: "read",
                title: "hero.png",
                state: "done",
                durationMs: 18,
                metadata: {
                  path: "D:/work/site/public/hero.png",
                  image: "image/png",
                  bytes: 84213,
                },
                args: { filePath: "D:/work/site/public/hero.png" },
                output: "It follows as a picture.",
              },
              {
                callId: "r5",
                name: "read",
                title: "tsconfig.json",
                state: "failed",
                durationMs: 1,
                metadata: { path: "D:/work/site/tsconfig.json", error: "No such file" },
                args: { filePath: "D:/work/site/tsconfig.json" },
                output: "",
              },
            ]}
          />
        </div>

        {/* The other half of the conversation. What a person pastes in is
            markdown too - a brief, a checklist, a stack trace - and the bubble
            has to hold headings, lists and code without the padding coming
            apart at the first and last block. */}
        <p className="mb-2 text-xs text-muted-foreground">What the person wrote</p>
        <div className="mb-8 flex flex-col items-end">
          <div className="flex max-w-[80%] flex-col items-end">
            <div className="chat-measure rounded-2xl rounded-br-md bg-user-message-background px-4 py-2.5 wrap-break-word">
              <Markdown>{ASK}</Markdown>
            </div>
          </div>
        </div>
        <Markdown>{SAMPLE}</Markdown>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")).render(
  <StrictMode>
    {/* The app wraps everything in this, and a message that can save a picture
        needs it: the confirmation is a toast. */}
    <ToastProvider>
      <Page />
    </ToastProvider>
  </StrictMode>
);
