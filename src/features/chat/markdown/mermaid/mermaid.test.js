import { describe, expect, it } from "vitest";
import { parseMermaid } from "./parse.js";
import { layoutDiagram } from "./layout.js";

const flow = (source) => parseMermaid(source);

describe("reading a flowchart", () => {
  it("reads nodes, their shapes and the links between them", () => {
    const diagram = flow(`flowchart TD
  A[Start] --> B(Think)
  B --> C{Sure?}
  C -->|yes| D([Done])
  C -->|no| B`);

    expect(diagram.kind).toBe("flowchart");
    expect(diagram.direction).toBe("TD");
    expect(diagram.nodes.map((node) => [node.id, node.label, node.shape])).toEqual([
      ["A", "Start", "rect"],
      ["B", "Think", "round"],
      ["C", "Sure?", "diamond"],
      ["D", "Done", "stadium"],
    ]);
    expect(diagram.edges).toEqual([
      { from: "A", to: "B", stroke: "solid", head: "end", label: null },
      { from: "B", to: "C", stroke: "solid", head: "end", label: null },
      { from: "C", to: "D", stroke: "solid", head: "end", label: "yes" },
      { from: "C", to: "B", stroke: "solid", head: "end", label: "no" },
    ]);
  });

  it("reads the other spelling of an edge label", () => {
    const diagram = flow("graph LR\n  A -- takes a while --> B\n  B -. maybe .-> C\n  C == always ==> D");
    expect(diagram.edges.map((edge) => [edge.label, edge.stroke, edge.head])).toEqual([
      ["takes a while", "solid", "end"],
      ["maybe", "dotted", "end"],
      ["always", "thick", "end"],
    ]);
  });

  it("follows a chain written on one line", () => {
    const diagram = flow("graph LR\n  A --> B --> C");
    expect(diagram.edges.map((edge) => `${edge.from}${edge.to}`)).toEqual(["AB", "BC"]);
    expect(diagram.nodes).toHaveLength(3);
  });

  it("takes the label from wherever the node was first written out", () => {
    // Mermaid lets a node appear bare and be labelled later, which is how
    // models usually write the second half of a diagram.
    const diagram = flow("flowchart TD\n  A --> B\n  B[Named later]\n  A[Named too]");
    expect(diagram.nodes.map((node) => node.label)).toEqual(["Named too", "Named later"]);
  });

  it("keeps every shape it knows apart", () => {
    const diagram = flow(`flowchart LR
  a[rect] --> b(round)
  b --> c([stadium])
  c --> d[[subroutine]]
  d --> e[(store)]
  e --> f((circle))
  f --> g{diamond}
  g --> h{{hexagon}}
  h --> i[/slanted/]`);
    expect(diagram.nodes.map((node) => node.shape)).toEqual([
      "rect",
      "round",
      "stadium",
      "subroutine",
      "cylinder",
      "circle",
      "diamond",
      "hexagon",
      "parallelogram",
    ]);
  });

  it("groups the nodes of a subgraph, and ignores styling", () => {
    const diagram = flow(`flowchart TD
  %% a comment
  subgraph Backend
    api[API] --> db[(Postgres)]
  end
  web[Web] --> api
  classDef pale fill:#eee
  class web pale
  style api stroke:#333`);

    expect(diagram.groups).toEqual([{ id: "Backend", label: "Backend", nodes: ["api", "db"] }]);
    expect(diagram.nodes.map((node) => node.id)).toEqual(["api", "db", "web"]);
    expect(diagram.edges).toHaveLength(2);
  });

  it("reads a line break in a label", () => {
    const diagram = flow('flowchart TD\n  A["one<br/>two"] --> B');
    expect(diagram.nodes[0].label).toBe("one\ntwo");
  });

  it("gives back nothing rather than half a diagram", () => {
    expect(flow("classDiagram\n  Animal <|-- Duck")).toBe(null);
    expect(flow("gantt\n  title A")).toBe(null);
    expect(flow("flowchart TD\n  A --> ")).toBe(null);
    expect(flow("")).toBe(null);
    expect(flow("not a diagram at all")).toBe(null);
  });
});

describe("reading a sequence diagram", () => {
  const diagram = parseMermaid(`sequenceDiagram
  participant U as User
  participant S as Server
  U->>S: GET /thing
  S-->>U: 200 OK
  Note right of S: cached
  S->>S: revalidate`);

  it("collects the actors, named as they were introduced", () => {
    expect(diagram.kind).toBe("sequence");
    expect(diagram.actors).toEqual([
      { id: "U", label: "User" },
      { id: "S", label: "Server" },
    ]);
  });

  it("reads messages, their direction, their line and their text", () => {
    const messages = diagram.steps.filter((step) => step.type === "message");
    expect(messages.map((step) => [step.from, step.to, step.stroke, step.label])).toEqual([
      ["U", "S", "solid", "GET /thing"],
      ["S", "U", "dashed", "200 OK"],
      ["S", "S", "solid", "revalidate"],
    ]);
  });

  it("reads a note", () => {
    expect(diagram.steps.find((step) => step.type === "note")).toEqual({
      type: "note",
      side: "right",
      actors: ["S"],
      label: "cached",
    });
  });

  it("declares an actor that was only ever used", () => {
    const bare = parseMermaid("sequenceDiagram\n  Alice->>Bob: hi");
    expect(bare.actors.map((actor) => actor.id)).toEqual(["Alice", "Bob"]);
  });

  it("keeps a loop as a marker rather than refusing the diagram", () => {
    const looped = parseMermaid("sequenceDiagram\n  loop every minute\n    A->>B: poll\n  end");
    expect(looped.steps[0]).toEqual({ type: "block", keyword: "loop", label: "every minute" });
    expect(looped.steps[1].type).toBe("message");
  });
});

describe("laying a flowchart out", () => {
  const diagram = layoutDiagram(
    flow("flowchart TD\n  A[Start] --> B[Middle]\n  A --> C[Other]\n  B --> D[End]\n  C --> D")
  );

  it("puts every node somewhere inside the canvas", () => {
    expect(diagram.nodes).toHaveLength(4);
    for (const node of diagram.nodes) {
      expect(node.w).toBeGreaterThan(0);
      expect(node.h).toBeGreaterThan(0);
      expect(node.x).toBeGreaterThanOrEqual(diagram.viewBox.x);
      expect(node.y).toBeGreaterThanOrEqual(diagram.viewBox.y);
      expect(node.x + node.w).toBeLessThanOrEqual(diagram.viewBox.x + diagram.viewBox.w);
      expect(node.y + node.h).toBeLessThanOrEqual(diagram.viewBox.y + diagram.viewBox.h);
    }
  });

  it("puts a node below the one it comes from, and siblings side by side", () => {
    const at = Object.fromEntries(diagram.nodes.map((node) => [node.id, node]));
    expect(at.A.cy).toBeLessThan(at.B.cy);
    expect(at.B.cy).toBeLessThan(at.D.cy);
    expect(at.B.cy).toBe(at.C.cy);
    expect(at.B.cx).not.toBe(at.C.cx);
  });

  it("never overlaps two boxes", () => {
    for (const a of diagram.nodes) {
      for (const b of diagram.nodes) {
        if (a.id >= b.id) continue;
        const apart =
          a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
        expect(apart, `${a.id} and ${b.id} overlap`).toBe(true);
      }
    }
  });

  it("draws every edge as a path with an end point", () => {
    expect(diagram.edges).toHaveLength(4);
    for (const edge of diagram.edges) {
      expect(edge.path.startsWith("M ")).toBe(true);
      expect(Number.isFinite(edge.endX)).toBe(true);
      expect(Number.isFinite(edge.endY)).toBe(true);
    }
  });

  it("runs left to right when it was asked to", () => {
    const sideways = layoutDiagram(flow("flowchart LR\n  A --> B"));
    const at = Object.fromEntries(sideways.nodes.map((node) => [node.id, node]));
    expect(at.A.cx).toBeLessThan(at.B.cx);
    expect(at.A.cy).toBe(at.B.cy);
  });

  it("runs bottom to top when it was asked to", () => {
    const upward = layoutDiagram(flow("flowchart BT\n  A --> B"));
    const at = Object.fromEntries(upward.nodes.map((node) => [node.id, node]));
    expect(at.A.cy).toBeGreaterThan(at.B.cy);
  });

  it("survives a cycle, which a longest path alone would not", () => {
    const looped = layoutDiagram(flow("flowchart TD\n  A --> B\n  B --> C\n  C --> A"));
    expect(looped.nodes).toHaveLength(3);
    expect(looped.edges).toHaveLength(3);
  });

  it("draws a self-loop as its own curve", () => {
    const self = layoutDiagram(flow("flowchart TD\n  A --> A"));
    expect(self.edges).toHaveLength(1);
    expect(self.edges[0].path).toMatch(/^M .* C /);
  });

  it("boxes a subgraph around the nodes in it", () => {
    const grouped = layoutDiagram(
      flow("flowchart TD\n  subgraph Api\n    a[One] --> b[Two]\n  end\n  c[Out] --> a")
    );
    const [group] = grouped.groups;
    const members = grouped.nodes.filter((node) => ["a", "b"].includes(node.id));
    for (const node of members) {
      expect(node.x).toBeGreaterThanOrEqual(group.x);
      expect(node.y).toBeGreaterThanOrEqual(group.y);
      expect(node.x + node.w).toBeLessThanOrEqual(group.x + group.w);
      expect(node.y + node.h).toBeLessThanOrEqual(group.y + group.h);
    }
  });
});

describe("laying a sequence diagram out", () => {
  const diagram = layoutDiagram(
    parseMermaid(`sequenceDiagram
  participant A as Alice
  participant B as Bob
  A->>B: a message long enough to need room
  B-->>A: fine
  Note over A,B: they agree`)
  );

  it("spreads the actors far enough apart for the longest message", () => {
    const [a, b] = diagram.actors;
    expect(b.cx - a.cx).toBeGreaterThan("a message long enough to need room".length * 6.9);
  });

  it("stacks the steps down the page, in order", () => {
    const ys = diagram.steps.map((step) => step.y);
    expect([...ys].sort((x, y) => x - y)).toEqual(ys);
    expect(diagram.steps.at(-1).y + 20).toBeLessThan(diagram.height);
  });

  it("puts a note beside the lifeline when it was told which side", () => {
    const sided = layoutDiagram(
      parseMermaid("sequenceDiagram\n  A->>B: hi\n  Note right of A: waiting")
    );
    const note = sided.steps.find((step) => step.type === "note");
    const [a] = sided.actors;
    expect(note.x).toBeGreaterThan(a.cx);
  });

  it("puts a note over both lifelines it names", () => {
    const note = diagram.steps.find((step) => step.type === "note");
    const [a, b] = diagram.actors;
    expect(note.x).toBeLessThan(a.cx);
    expect(note.x + note.w).toBeGreaterThan(b.cx);
  });

  it("has nothing to lay out for a diagram it could not read", () => {
    expect(layoutDiagram(null)).toBe(null);
  });
});
